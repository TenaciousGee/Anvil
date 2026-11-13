// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Turn a probed source and a resolved preset into an ordered job plan.
//!
//! The plan is pure data: files to write, commands to run and the
//! dependencies between them. The runner (in the app) executes it; the CLI
//! prints it. Nothing here touches the filesystem or spawns processes.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::audio::{default_audio_plan, AudioAction, AudioPlan, AudioTarget};
use crate::chapters::ChapterPlan;
use crate::command::Command;
use crate::crop::Crop;
use crate::encoder::{self, EncodeInputs, EncoderKind};
use crate::model::{DynamicRange, ResolutionClass, SourceInfo};
use crate::preset::Preset;
use crate::scriptgen::{self, FieldOrder, FilterBlock, FilterProfile, ScriptContext};
use crate::sizing::{self, Budget, QualitySearch, CRF_PLACEHOLDER};
use crate::subtitles::{default_subtitle_plan, SubSource, SubtitlePlan};
use crate::trackmap::TrackIdMap;
use crate::verify::Expectations;
use crate::Result;

/// Per-job choices made in the UI on top of the preset.
#[derive(Debug, Clone, Default)]
pub struct PlanOptions {
    pub work_dir: PathBuf,
    pub output: PathBuf,
    /// Crop from auto-crop or the preview. `None` means no crop.
    pub crop: Option<Crop>,
    /// The user's per-file audio order. `None` uses the default rule.
    pub audio_override: Option<Vec<AudioPlan>>,
    /// The user's subtitle tracks. `None` uses the preset's rule.
    pub subtitle_override: Option<Vec<SubtitlePlan>>,
    /// The user's chapters. `None` uses the preset (keep or drop).
    pub chapters: Option<ChapterPlan>,
    /// The user's filter chain. `None` uses the preset's.
    pub filters: Option<FilterProfile>,
    /// ffprobe index -> mkvmerge track ID, from `mkvmerge -J`. Needed for
    /// non-Matroska sources; `None` assumes they are equal.
    pub track_ids: Option<TrackIdMap>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    #[default]
    WriteFile,
    ExtractMetadata,
    ExtractTimestamps,
    /// Measure a track's loudness (EBU R128) and work out the gain.
    MeasureLoudness,
    /// Copy a subtitle stream out of the source.
    ExtractSubtitle,
    /// Keep only the forced captions of a PGS stream (built in).
    FilterSubtitles,
    /// Read image subtitles as text with Tesseract (built in).
    OcrSubtitles,
    /// Dolby Vision L5 offsets for the crop (built in).
    AdjustActiveArea,
    /// Sample encodes that pick the CRF for a size target.
    ChooseQuality,
    EncodeVideo,
    EncodeAudio,
    Mux,
    Verify,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Step {
    pub label: String,
    pub kind: StepKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Command>,
    /// For `WriteFile` steps: destination and contents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<(PathBuf, String)>,
    /// Indices of steps that must finish first. Steps with no unmet
    /// dependencies may run in parallel (audio alongside video).
    pub depends_on: Vec<usize>,
    /// For `ChooseQuality` steps: the sample encodes and the size target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<QualitySearch>,
    /// Read the CRF chosen by a `ChooseQuality` step from this file and put
    /// it in place of [`crate::sizing::CRF_PLACEHOLDER`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crf_from: Option<PathBuf>,
    /// Values worked out by earlier steps: each placeholder in the command
    /// is replaced with the (trimmed) contents of its file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<Substitution>,
    /// For `MeasureLoudness` steps: the target, and where the gain goes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness: Option<LoudnessMeasure>,
    /// For `FilterSubtitles` steps: PGS in, forced captions only out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forced_subs: Option<(PathBuf, PathBuf)>,
    /// For `OcrSubtitles` steps: image subtitles in, SRT out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr: Option<OcrJob>,
    /// For `AdjustActiveArea` steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_area: Option<ActiveAreaJob>,
}

/// Work out the Dolby Vision L5 for a crop: export the source RPU's L5,
/// subtract the crop, and write the editor config and the expectation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveAreaJob {
    pub rpu: PathBuf,
    pub crop: Crop,
    pub edit_json: PathBuf,
    pub expected: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrJob {
    /// PGS (.sup) or VobSub (.idx with its .sub).
    pub input: PathBuf,
    pub output: PathBuf,
    pub language: Option<String>,
    pub forced_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Substitution {
    pub placeholder: String,
    pub file: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoudnessMeasure {
    pub target_lufs: f64,
    pub max_true_peak_db: f64,
    /// Receives the gain in dB, e.g. "-1.85".
    pub result_file: PathBuf,
}

/// The fixed gain that brings a track to `target` LUFS without its true
/// peak passing `max_peak` dBTP.
pub fn loudness_gain(integrated: f64, true_peak: f64, target: f64, max_peak: f64) -> f64 {
    let gain = (target - integrated).min(max_peak - true_peak);
    (gain * 100.0).round() / 100.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobPlan {
    pub source: String,
    pub work_dir: PathBuf,
    pub output: PathBuf,
    pub encoder: Option<EncoderKind>,
    pub crop: Crop,
    pub output_size: (u32, u32),
    /// Filter chain in script order, for display ("Source (BestSource)", "Crop", ...).
    pub filters: Vec<String>,
    pub audio: Vec<AudioPlan>,
    pub subtitles: Vec<SubtitlePlan>,
    #[serde(default)]
    pub chapters: ChapterPlan,
    /// The filter chain in effect (the editor starts from it).
    #[serde(default)]
    pub profile: FilterProfile,
    /// Things the user should know before queueing: anything that would be
    /// dropped or changed.
    pub warnings: Vec<String>,
    /// Badges for the job card ("HDR10", "Dolby Vision 8.1", ...).
    pub badges: Vec<String>,
    /// Frames the video encoder should produce, for progress and ETA.
    #[serde(default)]
    pub expected_frames: Option<u64>,
    /// The size target, when the preset sets one.
    #[serde(default)]
    pub budget: Option<Budget>,
    /// What the output must contain; checked by the verify step.
    pub expect: Expectations,
    pub steps: Vec<Step>,
    /// Short timed encodes for "how long will this take?" (run on request).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_probe: Option<crate::estimate::SpeedProbe>,
}

/// FFmpeg commands that sample frames across the film for auto-crop. Each
/// command's stderr is parsed with [`crate::crop::parse_cropdetect`] and the
/// results combined with [`crate::crop::aggregate`].
pub fn autocrop_commands(source: &SourceInfo, samples: u32) -> Vec<Command> {
    autocrop_commands_with(source, samples, true)
}

/// As [`autocrop_commands`], choosing whether to decode keyframes only.
pub fn autocrop_commands_with(
    source: &SourceInfo,
    samples: u32,
    keyframes_only: bool,
) -> Vec<Command> {
    let duration = source.duration_secs.unwrap_or(0.0);
    if duration <= 0.0 || samples == 0 {
        return Vec::new();
    }
    // Skip the first and last 5% (logos, credits).
    (0..samples)
        .map(|i| {
            let t = duration * (0.05 + 0.9 * (f64::from(i) + 0.5) / f64::from(samples));
            Command::new("ffmpeg")
                .args(["-hide_banner", "-nostdin"])
                // Decode keyframes only: a seek into an M2TS (no index)
                // lands mid-GOP, and frames decoded without their
                // references come out grey, hiding the black bars.
                .args(if keyframes_only {
                    vec!["-skip_frame", "nokey"]
                } else {
                    vec![]
                })
                .args(["-ss", &format!("{t:.2}"), "-i", &source.path])
                .args(["-map", &format!("0:{}", source.video.index)])
                .args([
                    "-frames:v",
                    "5",
                    "-vf",
                    // A fractional limit scales with bit depth (24/255 on the
                    // 8-bit scale). A plain "24" is read on the 8-bit scale
                    // even for 10-bit HDR, where black sits at 64/1023, so
                    // letterbox bars on UHD sources were never detected.
                    "cropdetect=limit=0.094:round=2:reset=0",
                    "-f",
                    "null",
                    "-",
                ])
        })
        .collect()
}

pub fn plan_job(source: &SourceInfo, preset: &Preset, opts: &PlanOptions) -> Result<JobPlan> {
    let v = &source.video;
    let passthrough = preset.output.video_passthrough;
    let mut warnings = Vec::new();
    let mut badges = Vec::new();
    let mut steps: Vec<Step> = Vec::new();
    let src = source.path.clone();
    let work = |name: &str| -> String { opts.work_dir.join(name).to_string_lossy().into_owned() };

    let is_matroska = source.container.contains("matroska");
    if !is_matroska && opts.track_ids.is_none() {
        warnings.push(format!(
            "Source container is '{}'. Track IDs assume Matroska numbering; the runner re-maps them with mkvmerge -J before muxing.",
            source.container
        ));
    }
    // dovi_tool and hdr10plus_tool read raw HEVC or MKV. For anything else
    // (M2TS), FFmpeg demuxes the video stream into a pipe for them.
    let hevc_pipe = || -> Command {
        Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "error",
                "-i",
                &source.path,
            ])
            .args([
                "-map",
                "0:v:0",
                "-c:v",
                "copy",
                "-bsf:v",
                "hevc_mp4toannexb",
                "-f",
                "hevc",
                "-",
            ])
    };
    let hevc_input = |cmd: Command, args_before: &[&str], args_after: &[&str]| -> Command {
        let input = if is_matroska {
            source.path.as_str()
        } else {
            "-"
        };
        let cmd = cmd
            .args(args_before.iter().copied())
            .arg(input)
            .args(args_after.iter().copied());
        if is_matroska {
            cmd
        } else {
            cmd.piped_from(hevc_pipe())
        }
    };
    // mkvmerge track ID for an ffprobe stream index.
    let tid = |index: u32| -> u32 {
        opts.track_ids
            .as_ref()
            .and_then(|m| m.get(&index).copied())
            .unwrap_or(index)
    };

    // ---- Crop -------------------------------------------------------------
    let crop = if passthrough {
        Crop::default()
    } else {
        opts.crop.unwrap_or_default()
    };
    let output_size = crop.validate(v.width, v.height)?;
    let class = ResolutionClass::from_dimensions(output_size.0, output_size.1);

    // ---- Filters ----------------------------------------------------------
    // The job's own filter chain when the user edited it; otherwise the
    // preset's, with deinterlacing added for interlaced sources.
    let overridden = opts.filters.is_some();
    let mut profile = opts
        .filters
        .clone()
        .unwrap_or_else(|| preset.filters.clone());
    let has_field_filter = profile.blocks.iter().any(|b| b.runs_before_crop());
    if v.interlaced && !has_field_filter && !passthrough && !overridden {
        let field_order = if v.top_field_first {
            FieldOrder::TopFieldFirst
        } else {
            FieldOrder::BottomFieldFirst
        };
        profile
            .blocks
            .insert(0, FilterBlock::Deinterlace { field_order });
        warnings.push(
            "Interlaced source: deinterlacing (BWDIF) added. If the preview shows film content (telecine), switch the block to inverse telecine; for the finest result use QTGMC.".into(),
        );
    }
    if profile.script.is_some() {
        warnings.push(
            "The filter script is hand-edited: crop and filter changes no longer reach it, and it must keep the frame count for Dolby Vision and HDR10+ to line up.".into(),
        );
    }
    let frame_count_changes = profile.changes_frame_count();

    // ---- Badges -----------------------------------------------------------
    match v.dynamic_range {
        crate::model::DynamicRange::Hdr10 => badges.push("HDR10".to_string()),
        crate::model::DynamicRange::Hlg => badges.push("HLG".to_string()),
        crate::model::DynamicRange::Sdr => {}
    }

    // ---- Encoder choice and HDR metadata ----------------------------------
    let encoder_kind = (!passthrough).then_some(preset.video.encoder);
    let out_codec = preset.video.codec();
    if let Some(kind) = encoder_kind {
        encoder::preflight(kind, &out_codec, v)?;
    }

    let mut rpu_path = None;
    let mut rpu_step = None;
    let mut dv_l5: Option<PathBuf> = None;
    if let Some(dv) = &v.dolby_vision {
        let keep = preset.hdr.keep_dolby_vision;
        if passthrough {
            badges.push(format!("Dolby Vision {} (copied)", dv.profile));
        } else if !keep {
            warnings.push("Dolby Vision metadata will be dropped (preset setting).".into());
        } else if !encoder_kind.is_some_and(|k| k.supports_dolby_vision(&out_codec)) {
            warnings.push(format!(
                "Dolby Vision metadata will be dropped: {} ({}) cannot carry it. HDR10 is kept.",
                preset.video.encoder.label(),
                out_codec.to_uppercase()
            ));
        } else if frame_count_changes {
            warnings.push("Dolby Vision metadata dropped: the filter profile changes the frame count, so the per-frame RPU would no longer line up.".into());
        } else {
            let raw = work("RPU.bin");
            let mut cmd = Command::new("dovi_tool");
            if dv.needs_profile_conversion() {
                // Mode 2: convert to profile 8.1 and discard the enhancement layer.
                cmd = cmd.args(["-m", "2"]);
                warnings.push(format!(
                    "Dolby Vision profile {}{} will be converted to single-layer profile 8.1.",
                    dv.profile,
                    if dv.has_enhancement_layer {
                        " (enhancement layer discarded)"
                    } else {
                        ""
                    }
                ));
            }
            cmd = hevc_input(cmd, &["extract-rpu"], &["-o", &raw]);
            steps.push(Step {
                label: "Extract Dolby Vision RPU".into(),
                kind: StepKind::ExtractMetadata,
                command: Some(cmd),
                file: None,
                depends_on: vec![],
                ..Default::default()
            });
            let mut last = steps.len() - 1;
            let mut final_rpu = raw.clone();
            if !crop.is_none() {
                // L5 (active area) offsets shrink by exactly what the crop
                // removes, scene by scene, worked out from the source's L5.
                let edit_json = work("rpu_edit.json");
                let expected = work("l5_expected.json");
                steps.push(Step {
                    label: "Work out the Dolby Vision active area for the crop".into(),
                    kind: StepKind::AdjustActiveArea,
                    depends_on: vec![last],
                    active_area: Some(ActiveAreaJob {
                        rpu: raw.clone().into(),
                        crop,
                        edit_json: edit_json.clone().into(),
                        expected: expected.clone().into(),
                    }),
                    ..Default::default()
                });
                dv_l5 = Some(PathBuf::from(expected));
                let edit_idx = steps.len() - 1;
                final_rpu = work("RPU_cropped.bin");
                steps.push(Step {
                    label: "Adjust RPU for crop".into(),
                    kind: StepKind::ExtractMetadata,
                    command: Some(
                        Command::new("dovi_tool")
                            .args(["editor", "-i", &raw, "-j", &edit_json, "-o", &final_rpu]),
                    ),
                    file: None,
                    depends_on: vec![last, edit_idx],
                    ..Default::default()
                });
                last = steps.len() - 1;
            }
            badges.push("Dolby Vision 8.1".into());
            rpu_path = Some(final_rpu);
            rpu_step = Some(last);
        }
    }

    let mut hdr10p_path = None;
    let mut hdr10p_step = None;
    if v.hdr10_plus {
        let keep = preset.hdr.keep_hdr10_plus;
        if passthrough {
            badges.push("HDR10+ (copied)".into());
        } else if !keep {
            warnings.push("HDR10+ metadata will be dropped (preset setting).".into());
        } else if !encoder_kind.is_some_and(|k| k.supports_hdr10_plus(&out_codec)) {
            warnings.push(format!(
                "HDR10+ metadata will be dropped: {} ({}) cannot carry it. HDR10 is kept.",
                preset.video.encoder.label(),
                out_codec.to_uppercase()
            ));
        } else if frame_count_changes {
            warnings.push(
                "HDR10+ metadata dropped: the filter profile changes the frame count.".into(),
            );
        } else {
            let json = work("hdr10plus.json");
            steps.push(Step {
                label: "Extract HDR10+ metadata".into(),
                kind: StepKind::ExtractMetadata,
                command: Some(hevc_input(
                    Command::new("hdr10plus_tool"),
                    &["extract"],
                    &["-o", &json],
                )),
                file: None,
                depends_on: vec![],
                ..Default::default()
            });
            badges.push("HDR10+".into());
            hdr10p_path = Some(json);
            hdr10p_step = Some(steps.len() - 1);
        }
    }

    // ---- VFR timecodes ----------------------------------------------------
    let mut timestamps = None;
    let mut ts_step = None;
    if v.vfr_suspected && !passthrough {
        if !is_matroska {
            warnings.push("Source may be variable frame rate, but timecodes can only be carried over from MKV sources; output will be constant frame rate.".into());
        } else if frame_count_changes {
            warnings.push("Source looks variable frame rate, but the filters change the frame count; output will be constant frame rate.".into());
        } else {
            let ts = work("timestamps_v2.txt");
            steps.push(Step {
                label: "Extract VFR timecodes".into(),
                kind: StepKind::ExtractTimestamps,
                command: Some(Command::new("mkvextract").args([
                    src.clone(),
                    "timestamps_v2".into(),
                    format!("{}:{ts}", v.index),
                ])),
                file: None,
                depends_on: vec![],
                ..Default::default()
            });
            badges.push("VFR".into());
            timestamps = Some(ts);
            ts_step = Some(steps.len() - 1);
        }
    }

    // ---- Audio and subtitles kept (the size budget needs them) -----------
    let audio = opts
        .audio_override
        .clone()
        .unwrap_or_else(|| default_audio_plan(&source.audio, &preset.audio));

    let subtitles: Vec<SubtitlePlan> = opts
        .subtitle_override
        .clone()
        .unwrap_or_else(|| default_subtitle_plan(&source.subtitles, &preset.subtitles));

    // ---- Video ------------------------------------------------------------
    let fps = v.avg_frame_rate.or(v.frame_rate).map(|r| r.as_f64());
    let mut budget: Option<Budget> = None;
    let mut speed_probe: Option<crate::estimate::SpeedProbe> = None;
    let mut video_file = None;
    let mut video_step = None;
    if let Some(kind) = encoder_kind {
        let script_path = work("job.vpy");
        let script = scriptgen::render(
            &profile,
            &ScriptContext {
                source_path: src.clone(),
                crop,
                output_bit_depth: 10,
            },
        );
        steps.push(Step {
            label: "Write VapourSynth script".into(),
            kind: StepKind::WriteFile,
            command: None,
            file: Some((PathBuf::from(&script_path), script)),
            depends_on: vec![],
            ..Default::default()
        });
        let script_step = steps.len() - 1;

        // Crop-only jobs (the default) are fed straight from FFmpeg's
        // multithreaded decoder. A VapourSynth source filter adds nothing
        // there, and BestSource first reads the whole file to index it.
        let direct = profile.blocks.is_empty() && profile.script.is_none();
        let decode = |range: Option<(f64, u64)>| -> Command {
            if direct {
                let mut c =
                    Command::new("ffmpeg").args(["-hide_banner", "-nostdin", "-loglevel", "error"]);
                if let Some((t, _)) = range {
                    // Seek just short of the sample and decode into it, so
                    // the first frames have their references (M2TS has no
                    // index and lands mid-GOP).
                    let lead = t.min(2.0);
                    c = c.args(["-ss", &format!("{:.3}", t - lead)]);
                    c = c.args(["-i", &src, "-ss", &format!("{lead:.3}")]);
                } else {
                    c = c.args(["-i", &src]);
                }
                c = c.args(["-map", &format!("0:{}", v.index)]);
                if let Some((_, frames)) = range {
                    c = c.args(["-frames:v", &frames.to_string()]);
                }
                if !crop.is_none() {
                    c = c.args([
                        "-vf",
                        &format!(
                            "crop={}:{}:{}:{}",
                            output_size.0, output_size.1, crop.left, crop.top
                        ),
                    ]);
                }
                // Every frame exactly once (the Dolby Vision RPU is per frame).
                c.args([
                    "-fps_mode",
                    "passthrough",
                    "-pix_fmt",
                    "yuv420p10le",
                    "-strict",
                    "-1",
                ])
                .args(["-f", "yuv4mpegpipe", "-"])
            } else {
                let mut c = Command::new("vspipe").args(["-c", "y4m"]);
                if let (Some((t, frames)), Some(fps)) = (range, fps) {
                    let first = (t * fps).round() as u64;
                    c = c.args([
                        "--start".to_string(),
                        first.to_string(),
                        "--end".to_string(),
                        (first + frames - 1).to_string(),
                    ]);
                }
                c.args([script_path.as_str(), "-"])
            }
        };

        let out = work(&format!("video.{}", encoder::output_extension(&out_codec)));
        let inputs = EncodeInputs {
            video: v,
            class,
            output: out.clone(),
            dolby_vision_rpu: rpu_path.clone(),
        };
        let build = |inputs: &EncodeInputs| match kind {
            EncoderKind::X265 => encoder::x265_command(&preset.video.x265, inputs),
            EncoderKind::SvtAv1 => encoder::svt_av1_command(&preset.video.svt_av1, inputs),
            EncoderKind::NvEncC => encoder::nvencc_command(&preset.video.nvencc, inputs),
            EncoderKind::QsvEncC => encoder::qsvencc_command(&preset.video.qsvencc, inputs),
            EncoderKind::VceEncC => encoder::vceencc_command(&preset.video.vceencc, inputs),
            EncoderKind::FfmpegVaapi => encoder::vaapi_command(&preset.video.ffmpeg_vaapi, inputs),
            EncoderKind::X264 => encoder::x264_command(&preset.video.x264, inputs),
            EncoderKind::Rav1e => encoder::rav1e_command(&preset.video.rav1e, inputs),
        };
        let mut enc = build(&inputs);
        if !kind.writes_static_hdr() && v.dynamic_range == DynamicRange::Hdr10 {
            warnings.push(format!(
                "{} signals HDR10 (PQ, BT.2020) but cannot write the mastering display and MaxCLL; most TVs still show it as HDR10, with a default brightness range.",
                kind.label()
            ));
        }

        // ---- Size target --------------------------------------------------
        let mut deps = vec![script_step];
        let mut crf_from = None;
        if let Some(percent) = preset.output.max_size_percent.filter(|p| *p > 0.0) {
            {
                let knob = sizing::knob(&preset.video, kind, class);
                match (sizing::budget(source, &audio, &subtitles, percent), fps) {
                    (None, _) | (_, None) => warnings.push(
                        "Size target skipped: the source's size, running time or frame rate is unknown.".into(),
                    ),
                    (Some(b), Some(fps)) if !sizing::worth_sampling(b.duration_secs * fps) => warnings.push(format!(
                        "The {percent}% size target is skipped for videos under about 22 minutes: sampling would take too long compared with the encode. Encoding at the preset's quality."
                    )),
                    (Some(b), _) if b.video_kbps < 1000.0 => warnings.push(format!(
                        "A {percent}% size target cannot be met: the kept audio and subtitles need {:.1} Mb/s of the {:.1} Mb/s available. Encoding at the preset's quality instead.",
                        (b.audio_kbps + b.subtitle_kbps) / 1000.0,
                        (b.video_kbps + b.audio_kbps + b.subtitle_kbps) / 1000.0
                    )),
                    (Some(b), Some(fps)) => {
                        if !b.estimated.is_empty() {
                            warnings.push(format!(
                                "The size target estimates the bitrate of {} (the container does not record it), so the final size may differ a little.",
                                b.estimated.join(", ")
                            ));
                        }
                        let base = knob.base;
                        let duration = b.duration_secs;
                        let mut samples = Vec::new();
                        let mut sample_outputs = Vec::new();
                        for (k, t) in sizing::sample_starts(duration, sizing::SAMPLE_COUNT)
                            .into_iter()
                            .enumerate()
                        {
                            let file = work(&format!(
                                "sample_{:02}.{}",
                                k + 1,
                                encoder::output_extension(&out_codec)
                            ));
                            let mut x = build(&EncodeInputs {
                                video: v,
                                class,
                                output: file.clone(),
                                dolby_vision_rpu: None,
                            });
                            if rpu_path.is_some() && kind == EncoderKind::X265 {
                                // Same rate limits as the real encode.
                                x = x.args(["--vbv-bufsize", "160000", "--vbv-maxrate", "160000"]);
                            }
                            samples.push(
                                sizing::with_quality(x, knob.flag, CRF_PLACEHOLDER)
                                    .piped_from(decode(Some((t, sizing::SAMPLE_FRAMES)))),
                            );
                            sample_outputs.push(file);
                        }
                        let result_file = PathBuf::from(work("crf.txt"));
                        steps.push(Step {
                            label: "Choose quality for the size target".into(),
                            kind: StepKind::ChooseQuality,
                            command: None,
                            file: None,
                            depends_on: vec![script_step],
                            quality: Some(QualitySearch {
                                base_crf: base,
                                max_crf: knob.max,
                                target_kbps: b.video_kbps,
                                per_halving: knob.per_halving,
                                step: knob.step,
                                label: knob.label.into(),
                                samples,
                                sample_outputs,
                                sample_secs: f64::from(sizing::SAMPLE_COUNT) * sizing::SAMPLE_FRAMES as f64 / fps,
                                result_file: result_file.clone(),
                            }),
                            ..Default::default()
                        });
                        deps.push(steps.len() - 1);
                        enc = sizing::with_quality(enc, knob.flag, CRF_PLACEHOLDER);
                        crf_from = Some(result_file);
                        badges.push(format!("Max {}", sizing::gb(b.target_bytes)));
                        budget = Some(b);
                    }
                }
            }
        }

        // ---- Speed estimate (run on request, before queueing) -------------
        if let (Some(fps), Some(secs)) = (fps, v.duration_secs.or(source.duration_secs)) {
            let total = (secs * fps).round() as u64;
            let mut samples = Vec::new();
            let mut outputs = Vec::new();
            // Two places in the film; at each a short and a long run, so the
            // start-up time (encoder set-up, seeking) cancels out.
            for (k, t) in sizing::sample_starts(secs, 2).into_iter().enumerate() {
                for (j, frames) in crate::estimate::RUN_FRAMES.into_iter().enumerate() {
                    let file = work(&format!(
                        "speed_{}{}.{}",
                        k + 1,
                        ["a", "b"][j],
                        encoder::output_extension(&out_codec)
                    ));
                    let mut x = build(&EncodeInputs {
                        video: v,
                        class,
                        output: file.clone(),
                        dolby_vision_rpu: None,
                    });
                    if rpu_path.is_some() && kind == EncoderKind::X265 {
                        x = x.args(["--vbv-bufsize", "160000", "--vbv-maxrate", "160000"]);
                    }
                    samples.push(x.piped_from(decode(Some((t, frames)))));
                    outputs.push(file);
                }
            }
            let script = steps[script_step].file.clone().filter(|_| !direct);
            speed_probe = Some(crate::estimate::SpeedProbe {
                samples,
                outputs,
                total_frames: total,
                sizing_frames: if budget.is_some() {
                    // One or two rounds of the sample encodes.
                    u64::from(sizing::SAMPLE_COUNT) * sizing::SAMPLE_FRAMES * 3 / 2
                } else {
                    0
                },
                script,
                encoder: kind.label().into(),
            });
        }

        deps.extend(rpu_step);
        steps.push(Step {
            label: format!("Encode video ({})", kind.label()),
            kind: StepKind::EncodeVideo,
            command: Some(enc.piped_from(decode(None))),
            file: None,
            depends_on: deps,
            crf_from,
            ..Default::default()
        });
        video_file = Some(out.clone());
        video_step = Some(steps.len() - 1);

        if let (Some(json), Some(extract)) = (&hdr10p_path, hdr10p_step) {
            let injected = work(&format!(
                "video.hdr10plus.{}",
                encoder::output_extension(&out_codec)
            ));
            steps.push(Step {
                label: "Inject HDR10+ metadata".into(),
                kind: StepKind::ExtractMetadata,
                command: Some(
                    Command::new("hdr10plus_tool")
                        .args(["inject", "-i", &out, "-j", json, "-o", &injected]),
                ),
                file: None,
                depends_on: vec![steps.len() - 1, extract],
                ..Default::default()
            });
            video_file = Some(injected);
            video_step = Some(steps.len() - 1);
        }
    }

    // ---- Audio ------------------------------------------------------------
    let mut encoded_audio: Vec<(usize, String, usize)> = Vec::new(); // (plan idx, file, step)
    for (i, a) in audio.iter().enumerate() {
        if let AudioAction::Encode {
            target,
            bitrate_kbps,
            channels,
        } = &a.action
        {
            let out = work(&format!("audio_{i}.{}", target.file_extension()));
            let map = format!("0:a:{}", a.source_audio_ordinal);
            let mut cmd = Command::new("ffmpeg")
                .args(["-hide_banner", "-nostdin", "-y", "-i", &src])
                .args(["-map", &map]);
            let mut deps = vec![];
            let mut inputs = vec![];
            if *target == AudioTarget::DtsCore {
                // The core is copied out of the DTS-HD stream, not re-encoded.
                cmd = cmd.args(["-c:a", "copy", "-bsf:a", "dca_core"]);
            } else {
                let mut filters = Vec::new();
                if let Some(lufs) = a.loudness_lufs {
                    let gain_file = PathBuf::from(work(&format!("audio_{i}.gain")));
                    steps.push(Step {
                        label: format!("Measure loudness of audio track {}", i + 1),
                        kind: StepKind::MeasureLoudness,
                        command: Some(
                            Command::new("ffmpeg")
                                .args(["-hide_banner", "-nostdin", "-i", &src, "-map", &map])
                                .args([
                                    "-af",
                                    "ebur128=peak=true:framelog=verbose",
                                    "-f",
                                    "null",
                                    "-",
                                ]),
                        ),
                        loudness: Some(LoudnessMeasure {
                            target_lufs: lufs,
                            max_true_peak_db: crate::audio::MAX_TRUE_PEAK_DB,
                            result_file: gain_file.clone(),
                        }),
                        ..Default::default()
                    });
                    deps.push(steps.len() - 1);
                    filters.push(format!("volume={GAIN_PLACEHOLDER}dB"));
                    inputs.push(Substitution {
                        placeholder: GAIN_PLACEHOLDER.into(),
                        file: gain_file,
                    });
                }
                // Encoders want plain layouts ("5.1", not "5.1(side)").
                filters.push(format!(
                    "aformat=channel_layouts={}",
                    ffmpeg_layout(*channels)
                ));
                cmd = cmd.args(["-af", &filters.join(",")]);
                cmd = cmd.args(["-c:a", target.ffmpeg_codec()]);
                if target.has_bitrate() {
                    cmd = cmd.args(["-b:a", &format!("{bitrate_kbps}k")]);
                }
            }
            steps.push(Step {
                label: format!("Encode audio track {} ({})", i + 1, a.describe()),
                kind: StepKind::EncodeAudio,
                command: Some(cmd.arg(out.clone())),
                depends_on: deps,
                inputs,
                ..Default::default()
            });
            encoded_audio.push((i, out, steps.len() - 1));
        }
    }
    for a in &audio {
        badges.push(short_audio_badge(a));
    }

    // ---- Subtitles made at encode time: forced-only and OCR ------------------
    let mut sub_files: Vec<(usize, String, usize)> = Vec::new(); // (plan idx, file, step)
    for (i, s) in subtitles.iter().enumerate() {
        if !(s.forced_only || s.ocr) {
            continue;
        }
        // The image subtitles to work on, and the step that makes them.
        let (images, made_by): (String, Option<usize>) = match &s.source {
            SubSource::File { path } => (path.clone(), None),
            SubSource::Track { index, ordinal } => {
                let vobsub = s.codec == crate::model::SubtitleCodec::VobSub;
                if vobsub && !is_matroska {
                    warnings.push(format!(
                        "Subtitle track {} is DVD VobSub in a file mkvextract cannot read, so it stays as images.",
                        i + 1
                    ));
                    continue;
                }
                let file = work(&format!("subs_{i}.{}", if vobsub { "idx" } else { "sup" }));
                let command = if vobsub {
                    Command::new("mkvextract")
                        .args([src.as_str(), "tracks"])
                        .arg(format!("{}:{file}", tid(*index)))
                } else {
                    Command::new("ffmpeg")
                        .args(["-hide_banner", "-nostdin", "-y", "-i", &src])
                        .args(["-map", &format!("0:s:{ordinal}"), "-c:s", "copy", &file])
                };
                steps.push(Step {
                    label: format!("Extract subtitle track {}", i + 1),
                    kind: StepKind::ExtractSubtitle,
                    command: Some(command),
                    ..Default::default()
                });
                (file, Some(steps.len() - 1))
            }
        };
        if s.ocr {
            let srt = work(&format!("subs_{i}.srt"));
            steps.push(Step {
                label: format!("Read subtitle track {} as text (OCR)", i + 1),
                kind: StepKind::OcrSubtitles,
                depends_on: made_by.into_iter().collect(),
                ocr: Some(OcrJob {
                    input: images.into(),
                    output: srt.clone().into(),
                    language: s.language.clone(),
                    forced_only: s.forced_only,
                }),
                ..Default::default()
            });
            sub_files.push((i, srt, steps.len() - 1));
        } else {
            let forced = work(&format!("subs_{i}.forced.sup"));
            steps.push(Step {
                label: format!("Keep forced captions of subtitle track {}", i + 1),
                kind: StepKind::FilterSubtitles,
                depends_on: made_by.into_iter().collect(),
                forced_subs: Some((images.into(), forced.clone().into())),
                ..Default::default()
            });
            sub_files.push((i, forced, steps.len() - 1));
        }
    }

    // ---- Chapters -------------------------------------------------------------
    let chapters = opts
        .chapters
        .clone()
        .unwrap_or(if preset.output.keep_chapters {
            ChapterPlan::Keep
        } else {
            ChapterPlan::Drop
        });
    let mut chapter_file = None;
    if let ChapterPlan::Custom { chapters: list } = &chapters {
        if list.is_empty() {
            warnings.push("The chapter list is empty, so the output will have no chapters.".into());
        } else {
            let path = PathBuf::from(work("chapters.xml"));
            let lang = audio
                .first()
                .and_then(|a| a.language.clone())
                .unwrap_or_else(|| "eng".into());
            steps.push(Step {
                label: "Write chapters".into(),
                kind: StepKind::WriteFile,
                file: Some((path.clone(), crate::chapters::to_matroska_xml(list, &lang))),
                ..Default::default()
            });
            chapter_file = Some((path, steps.len() - 1));
        }
    }

    // ---- Mux --------------------------------------------------------------
    let output = opts.output.to_string_lossy().into_owned();
    let mut mux = Command::new("mkvmerge").args(["-o", &output]);
    if let Some((path, _)) = &chapter_file {
        mux = mux.args(["--chapters", &path.to_string_lossy()]);
    }
    let mut track_order: Vec<String> = Vec::new();
    let mut file_idx = 0usize;

    if let Some(vf) = &video_file {
        if let Some(ts) = &timestamps {
            mux = mux.args(["--timestamps", &format!("0:{ts}")]);
        }
        mux = mux.arg(vf.clone());
        track_order.push(format!("{file_idx}:0"));
        file_idx += 1;
    }

    // The source file supplies passthrough audio, subtitles and chapters
    // (and the video itself in passthrough mode).
    let source_file_idx = file_idx;
    let pass_audio: Vec<&AudioPlan> = audio
        .iter()
        .filter(|a| a.action == AudioAction::Passthrough)
        .collect();
    if !passthrough {
        mux = mux.arg("--no-video");
    }
    mux = if pass_audio.is_empty() {
        mux.arg("--no-audio")
    } else {
        mux.args([
            "--audio-tracks",
            &join_ids(pass_audio.iter().map(|a| tid(a.source_index))),
        ])
    };
    let source_subs: Vec<(u32, &SubtitlePlan)> = subtitles
        .iter()
        .enumerate()
        .filter(|(i, _)| !sub_files.iter().any(|(pi, _, _)| pi == i))
        .filter_map(|(_, s)| match &s.source {
            SubSource::Track { index, .. } => Some((tid(*index), s)),
            SubSource::File { .. } => None,
        })
        .collect();
    mux = if source_subs.is_empty() {
        mux.arg("--no-subtitles")
    } else {
        let ids = join_ids(source_subs.iter().map(|(id, _)| *id));
        let mut m = mux.args(["--subtitle-tracks", &ids]);
        for (id, s) in &source_subs {
            m = flag_subtitle(m, *id, s);
        }
        m
    };
    if chapters != ChapterPlan::Keep {
        // Dropped, or replaced by the edited list.
        mux = mux.arg("--no-chapters");
    }
    for a in &pass_audio {
        mux = flag_track(mux, tid(a.source_index), a);
    }
    mux = mux.arg(src.clone());
    file_idx += 1;

    if passthrough {
        track_order.push(format!("{source_file_idx}:{}", tid(v.index)));
    }

    let mut audio_order: Vec<String> = Vec::new();
    for (i, a) in audio.iter().enumerate() {
        match encoded_audio.iter().find(|(pi, _, _)| *pi == i) {
            Some((_, file, _)) => {
                mux = flag_track(mux, 0, a).arg(file.clone());
                audio_order.push(format!("{file_idx}:0"));
                file_idx += 1;
            }
            None => audio_order.push(format!("{source_file_idx}:{}", tid(a.source_index))),
        }
    }
    track_order.extend(audio_order);
    for (i, s) in subtitles.iter().enumerate() {
        let made = sub_files
            .iter()
            .find(|(pi, _, _)| *pi == i)
            .map(|(_, f, _)| f.clone());
        let file = match (made, &s.source) {
            (Some(f), _) => f,
            (None, SubSource::File { path }) => path.clone(),
            (None, SubSource::Track { index, .. }) => {
                track_order.push(format!("{source_file_idx}:{}", tid(*index)));
                continue;
            }
        };
        mux = flag_subtitle(mux, 0, s).arg(file);
        track_order.push(format!("{file_idx}:0"));
        file_idx += 1;
    }
    mux = mux.args(["--track-order", &track_order.join(",")]);

    let mut mux_deps: Vec<usize> = encoded_audio.iter().map(|(_, _, s)| *s).collect();
    mux_deps.extend(sub_files.iter().map(|(_, _, s)| *s));
    mux_deps.extend(chapter_file.as_ref().map(|(_, s)| *s));
    mux_deps.extend(video_step);
    mux_deps.extend(ts_step);
    steps.push(Step {
        label: "Mux to MKV".into(),
        kind: StepKind::Mux,
        command: Some(mux),
        file: None,
        depends_on: mux_deps,
        ..Default::default()
    });
    let mux_step = steps.len() - 1;

    steps.push(Step {
        label: "Verify output".into(),
        kind: StepKind::Verify,
        command: Some(Command::new("ffprobe").args(crate::probe::ffprobe_args(&output))),
        file: None,
        depends_on: vec![mux_step],
        ..Default::default()
    });

    let mut filters = vec![if profile.blocks.is_empty() {
        "Source (FFmpeg, direct)".to_string()
    } else {
        format!("Source ({:?})", profile.source)
    }];
    if encoder_kind.is_some() {
        filters.extend(
            profile
                .blocks
                .iter()
                .filter(|b| b.runs_before_crop())
                .map(FilterBlock::label),
        );
        if !crop.is_none() {
            filters.push(format!("Crop to {}x{}", output_size.0, output_size.1));
        }
        filters.extend(
            profile
                .blocks
                .iter()
                .filter(|b| !b.runs_before_crop())
                .map(FilterBlock::label),
        );
        filters.push("Convert to 10-bit".into());
    } else {
        filters = vec!["None (video passthrough)".into()];
    }

    // ---- Expectations for the verify step ------------------------------------
    let ivtc = profile
        .blocks
        .iter()
        .any(|b| matches!(b, FilterBlock::Ivtc { .. }));
    // The video stream's own length when known, else the file's.
    let video_secs = v.duration_secs.or(source.duration_secs);
    let expected_frames = match (video_secs, fps) {
        (Some(d), Some(f)) if encoder_kind.is_some() => {
            let frames = d * f * if ivtc { 0.8 } else { 1.0 };
            Some(frames.round() as u64)
        }
        _ => None,
    };
    let carries_static_hdr = encoder_kind.is_some_and(EncoderKind::writes_static_hdr);
    let hdr10 = v.hdr10.unwrap_or_default();
    let expect = Expectations {
        duration_secs: video_secs,
        width: output_size.0,
        height: output_size.1,
        bit_depth: if let Some(k) = encoder_kind {
            k.output_depth(&out_codec)
        } else {
            v.bit_depth
        },
        video_codec: match encoder_kind {
            None => v.codec.clone(),
            Some(_) => out_codec.clone(),
        },
        dynamic_range: v.dynamic_range,
        mastering_display: hdr10.mastering_display.is_some() && (passthrough || carries_static_hdr),
        content_light: hdr10.content_light.is_some() && (passthrough || carries_static_hdr),
        dolby_vision_profile: if passthrough {
            v.dolby_vision.as_ref().map(|d| d.profile)
        } else {
            rpu_path.as_ref().map(|_| 8)
        },
        hdr10_plus: if passthrough {
            v.hdr10_plus
        } else {
            hdr10p_path.is_some()
        },
        audio: audio.iter().map(AudioPlan::output_codec).collect(),
        subtitles: subtitles.len(),
        chapters: match &chapters {
            ChapterPlan::Keep => source.chapter_count > 0,
            ChapterPlan::Drop => false,
            ChapterPlan::Custom { chapters } => !chapters.is_empty(),
        },
        chapter_count: Some(match &chapters {
            ChapterPlan::Keep => source.chapter_count,
            ChapterPlan::Drop => 0,
            ChapterPlan::Custom { chapters } => chapters.len(),
        }),
        max_bytes: budget.as_ref().map(|b| b.target_bytes),
        dv_l5: dv_l5.clone(),
        hdr10plus_source: hdr10p_path.as_ref().map(PathBuf::from),
    };

    Ok(JobPlan {
        source: src,
        expected_frames,
        budget,
        expect,
        work_dir: opts.work_dir.clone(),
        filters,
        output: opts.output.clone(),
        encoder: encoder_kind,
        crop,
        output_size,
        audio,
        subtitles,
        chapters,
        profile: profile.clone(),
        warnings,
        badges,
        steps,
        speed_probe,
    })
}

/// Render a plan as a standalone bash script that runs every step in order.
///
/// This lets jobs be encoded today, before the built-in runner exists, and
/// gives a reproducible record of exactly what a job does.
pub fn to_shell_script(plan: &JobPlan) -> String {
    use crate::command::shell_quote;
    let mut s = String::new();
    s.push_str("#!/usr/bin/env bash\n");
    s.push_str("# Generated by Anvil. Runs every step of one job in order.\n");
    s.push_str(&format!(
        "# Source: {}\n# Output: {}\n",
        plan.source,
        plan.output.display()
    ));
    for w in &plan.warnings {
        s.push_str(&format!("# Note: {w}\n"));
    }
    s.push_str("set -euo pipefail\n");
    s.push_str("# VapourSynth R80+ finds Anvil's plugins through this variable.\n");
    s.push_str(&format!(
        "export {0}=\"${{{0}:-${{XDG_DATA_HOME:-$HOME/.local/share}}/anvil/vs/vapoursynth/plugins}}\"\n\n",
        crate::paths::VS_EXTRA_PLUGIN_ENV
    ));

    let mut dirs: Vec<String> = vec![plan.work_dir.to_string_lossy().into_owned()];
    if let Some(parent) = plan.output.parent().filter(|p| !p.as_os_str().is_empty()) {
        dirs.push(parent.to_string_lossy().into_owned());
    }
    dirs.dedup();
    s.push_str(&format!(
        "mkdir -p {}\n",
        dirs.iter()
            .map(|d| shell_quote(d))
            .collect::<Vec<_>>()
            .join(" ")
    ));

    let total = plan.steps.len();
    for (i, step) in plan.steps.iter().enumerate() {
        s.push_str(&format!(
            "\necho '==> [{}/{}] {}'\n",
            i + 1,
            total,
            step.label.replace('\'', "")
        ));
        if let Some((path, contents)) = &step.file {
            s.push_str(&format!(
                "cat > {} <<'ANVIL_EOF'\n{}",
                shell_quote(&path.to_string_lossy()),
                contents
            ));
            if !contents.ends_with('\n') {
                s.push('\n');
            }
            s.push_str("ANVIL_EOF\n");
        }
        if let Some(q) = &step.quality {
            s.push_str(&quality_search_shell(q));
        }
        if let Some(a) = &step.active_area {
            s.push_str(&format!(
                "# Anvil subtracts the crop from the source's L5 offsets; this script zeroes them.\nprintf '{{\"active_area\": {{\"crop\": true}}}}\\n' > {}\n",
                shell_quote(&a.edit_json.to_string_lossy())
            ));
            continue;
        }
        if let Some(o) = &step.ocr {
            s.push_str(&format!(
                "anvil ocr {} --lang {}{} -o {}\n",
                shell_quote(&o.input.to_string_lossy()),
                shell_quote(crate::ocr::tesseract_language(o.language.as_deref())),
                if o.forced_only { " --forced-only" } else { "" },
                shell_quote(&o.output.to_string_lossy())
            ));
            continue;
        }
        if let Some((full, forced)) = &step.forced_subs {
            s.push_str(&format!(
                "# Picking out forced captions is built into Anvil; this script keeps the whole track.\ncp {} {}\n",
                shell_quote(&full.to_string_lossy()),
                shell_quote(&forced.to_string_lossy())
            ));
            continue;
        }
        if let (Some(cmd), Some(m)) = (&step.command, &step.loudness) {
            // Measure, then work out the gain exactly as the runner does.
            s.push_str(&format!(
                "{} 2>&1 | awk '/Summary:/ {{ s = 1 }} s && $1 == \"I:\" {{ i = $2 }} s && $1 == \"Peak:\" {{ p = $2 }} END {{ g = {t} - i; c = {mp} - p; if (c < g) g = c; printf \"%.2f\\n\", g }}' > {}\n",
                cmd.to_shell(),
                shell_quote(&m.result_file.to_string_lossy()),
                t = m.target_lufs,
                mp = m.max_true_peak_db,
            ));
            continue;
        }
        if let Some(cmd) = &step.command {
            if let Some(path) = &step.crf_from {
                s.push_str(&format!(
                    "CRF=$(cat {})\n",
                    shell_quote(&path.to_string_lossy())
                ));
            }
            let mut line = with_shell_crf(&cmd.to_shell());
            for sub in &step.inputs {
                // The placeholder sits inside a single-quoted argument.
                line = line.replace(
                    &sub.placeholder,
                    &format!("'\"$(cat {})\"'", shell_quote(&sub.file.to_string_lossy())),
                );
            }
            s.push_str(&line);
            s.push('\n');
        }
    }
    s.push_str(&format!(
        "\necho 'Done: '{}\n",
        shell_quote(&plan.output.to_string_lossy())
    ));
    s
}

/// True when the encoder is fed straight from FFmpeg (crop-only jobs), so
/// the after-filters preview can use the same decode.
pub fn decodes_directly(plan: &JobPlan) -> bool {
    plan.steps
        .iter()
        .find(|s| s.kind == StepKind::EncodeVideo)
        .and_then(|s| s.command.as_ref())
        .and_then(|c| c.stdin_from.as_deref())
        .is_some_and(|up| up.program == "ffmpeg")
}

/// `{crf}` in a rendered command becomes the shell variable.
fn with_shell_crf(line: &str) -> String {
    line.replace(&crate::command::shell_quote(CRF_PLACEHOLDER), "\"$CRF\"")
}

/// The size-target search as bash: the same two rounds as the built-in
/// runner, with awk for the arithmetic.
fn quality_search_shell(q: &QualitySearch) -> String {
    use crate::command::shell_quote;
    let mut s = String::from("sample_kbps() {\n    local bytes=0 f\n");
    for (cmd, out) in q.samples.iter().zip(&q.sample_outputs) {
        s.push_str(&format!("    {}\n", with_shell_crf(&cmd.to_shell())));
        let f = shell_quote(out);
        s.push_str(&format!(
            "    f=$(stat -c %s {f}); bytes=$((bytes + f)); rm -f {f}\n"
        ));
    }
    s.push_str(&format!(
        "    awk -v b=\"$bytes\" 'BEGIN {{ printf \"%.1f\", b * 8 / 1000 / {} }}'\n}}\n",
        q.sample_secs
    ));
    let (base, max, target) = (q.base_crf, q.max_crf, q.target_kbps);
    let (k, step, label) = (q.per_halving, q.step, &q.label);
    let (kmin, kmax) = (k / 3.0, k * 2.0);
    let per = 1.0 / step;
    s.push_str(&format!(
        r#"up() {{ awk -v x="$1" -v lo={base} -v hi={max} 'BEGIN {{ y = int(x * {per} + 0.999999) / {per}; if (y < lo) y = lo; if (y > hi) y = hi; print y }}'; }}
CRF={base}; B0=$(sample_kbps); echo "{label} $CRF: $B0 kb/s (target {target:.0})"
if awk -v b="$B0" 'BEGIN {{ exit !(b > {target}) }}'; then
    C0=$CRF; CRF=$(up "$(awk -v c="$C0" -v b="$B0" 'BEGIN {{ print c + {k} * log(b / {target}) / log(2) }}')")
    if [ "$CRF" != "$C0" ]; then
        B1=$(sample_kbps); echo "{label} $CRF: $B1 kb/s"
        CRF=$(up "$(awk -v c0="$C0" -v b0="$B0" -v c1="$CRF" -v b1="$B1" 'BEGIN {{
            k = {k}; if (b0 > b1 * 1.01) k = (c1 - c0) / (log(b0 / b1) / log(2));
            if (k < {kmin}) k = {kmin}; if (k > {kmax}) k = {kmax}; print c1 + k * log(b1 / {target}) / log(2) }}')")
    fi
fi
echo "Chosen {label}: $CRF"; echo "$CRF" > {}
"#,
        shell_quote(&q.result_file.to_string_lossy())
    ));
    s
}

/// Stands in for a measured gain until the loudness step has run.
pub const GAIN_PLACEHOLDER: &str = "{gain}";

/// FFmpeg's name for a plain channel layout of `n` channels.
fn ffmpeg_layout(n: u32) -> &'static str {
    match n {
        1 => "mono",
        2 => "stereo",
        3 => "2.1",
        4 => "quad",
        5 => "5.0",
        6 => "5.1",
        7 => "6.1",
        _ => "7.1",
    }
}

fn join_ids(ids: impl Iterator<Item = u32>) -> String {
    ids.map(|i| i.to_string()).collect::<Vec<_>>().join(",")
}

fn flag_track(mut c: Command, id: u32, a: &AudioPlan) -> Command {
    if let Some(lang) = &a.language {
        c = c.args(["--language", &format!("{id}:{lang}")]);
    }
    if let Some(title) = &a.title {
        c = c.args(["--track-name", &format!("{id}:{title}")]);
    }
    if a.delay_ms != 0 {
        c = c.args(["--sync", &format!("{id}:{}", a.delay_ms)]);
    }
    c.args([
        "--default-track-flag",
        &format!("{id}:{}", if a.default { "yes" } else { "no" }),
    ])
}

fn flag_subtitle(mut c: Command, id: u32, s: &SubtitlePlan) -> Command {
    if let Some(lang) = &s.language {
        c = c.args(["--language", &format!("{id}:{lang}")]);
    }
    if let Some(title) = &s.title {
        c = c.args(["--track-name", &format!("{id}:{title}")]);
    }
    let yes = |b: bool| if b { "yes" } else { "no" };
    c.args(["--default-track-flag", &format!("{id}:{}", yes(s.default))])
        .args(["--forced-display-flag", &format!("{id}:{}", yes(s.forced))])
}

fn short_audio_badge(a: &AudioPlan) -> String {
    use crate::model::AudioCodec::*;
    match &a.action {
        AudioAction::Passthrough => match a.source_codec {
            TrueHd => "TrueHD".into(),
            DtsHdMa => "DTS-HD MA".into(),
            Lpcm => "LPCM".into(),
            Ac3 => "AC-3".into(),
            other => other.display_name().into(),
        },
        AudioAction::Encode { target, .. } => format!("{} (encoded)", target.short()),
    }
}

/// Per-job work folder: `<out_dir>/.anvil-work/<source stem>/`, so jobs
/// in a batch never share intermediate files.
pub fn default_work_dir(source: &Path, out_dir: &Path) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "job".into());
    out_dir.join(".anvil-work").join(stem)
}

/// Default output file name: source stem plus `.mkv`, next to `out_dir`.
pub fn default_output_path(source: &Path, out_dir: &Path) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".into());
    out_dir.join(format!("{stem}.anvil.mkv"))
}
