// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! End-to-end planner tests against ffprobe JSON fixtures.

use std::path::PathBuf;

use anvil_core::audio::{move_track, AudioAction, AudioTarget};
use anvil_core::crop::Crop;
use anvil_core::model::{AudioCodec, DynamicRange, ResolutionClass, SubtitleCodec};
use anvil_core::pipeline::{plan_job, PlanOptions, StepKind};
use anvil_core::preset::{PresetStore, DEFAULT_PRESET};
use anvil_core::probe::parse_ffprobe_json;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn opts(crop: Option<Crop>) -> PlanOptions {
    PlanOptions {
        work_dir: "/work/job1".into(),
        output: "/out/Example Film.mkv".into(),
        crop,
        audio_override: None,
        track_ids: None,
        ..Default::default()
    }
}

#[test]
fn uhd_probe_reads_everything() {
    let src = parse_ffprobe_json(&fixture("uhd_dovi_p7_truehd.json")).unwrap();
    let v = &src.video;
    assert_eq!((v.width, v.height, v.bit_depth), (3840, 2160, 10));
    assert_eq!(v.dynamic_range, DynamicRange::Hdr10);
    assert_eq!(v.resolution_class(), ResolutionClass::Uhd);
    let md = v.hdr10.unwrap().mastering_display.unwrap();
    assert_eq!(md.max_luminance, 4000.0);
    assert_eq!(v.hdr10.unwrap().content_light.unwrap().max_cll, 1105);
    let dv = v.dolby_vision.as_ref().unwrap();
    assert_eq!((dv.profile, dv.has_enhancement_layer), (7, true));
    assert!(v.hdr10_plus);
    assert!(!v.vfr_suspected && !v.interlaced);

    assert_eq!(src.audio.len(), 3);
    assert_eq!(src.audio[0].codec, AudioCodec::TrueHd);
    assert!(src.audio[0].object_audio);
    assert_eq!(src.audio[2].codec, AudioCodec::DtsHdMa);
    assert_eq!(src.subtitles[1].codec, SubtitleCodec::Pgs);
    assert!(src.subtitles[1].forced);
    assert_eq!(src.chapter_count, 3);
}

#[test]
fn uhd_quality_default_plan() {
    let src = parse_ffprobe_json(&fixture("uhd_dovi_p7_truehd.json")).unwrap();
    let preset = PresetStore::builtin()
        .unwrap()
        .resolve(DEFAULT_PRESET, None)
        .unwrap();
    let crop = Crop {
        top: 276,
        bottom: 276,
        ..Default::default()
    };
    let plan = plan_job(&src, &preset, &opts(Some(crop))).unwrap();

    assert_eq!(plan.output_size, (3840, 1608));

    // Audio: TrueHD passthrough first, disc AC-3 passthrough second.
    assert_eq!(plan.audio.len(), 2);
    assert_eq!(plan.audio[0].source_codec, AudioCodec::TrueHd);
    assert_eq!(plan.audio[1].source_codec, AudioCodec::Ac3);
    assert!(plan
        .audio
        .iter()
        .all(|a| a.action == AudioAction::Passthrough));

    // Subtitles: every language kept (empty language list).
    assert_eq!(plan.subtitles.len(), 3);

    let kinds: Vec<&StepKind> = plan.steps.iter().map(|s| &s.kind).collect();
    assert!(kinds.contains(&&StepKind::EncodeVideo));
    assert!(
        !kinds.contains(&&StepKind::EncodeAudio),
        "nothing to encode"
    );

    let shell: Vec<String> = plan
        .steps
        .iter()
        .filter_map(|s| s.command.as_ref().map(|c| c.to_shell()))
        .collect();
    let all = shell.join("\n");
    println!("{all}");

    // Dolby Vision profile 7 converted to 8.1, cropped, and injected by x265.
    assert!(all.contains("dovi_tool -m 2 extract-rpu"));
    assert!(all.contains("dovi_tool editor -i /work/job1/RPU.bin -j /work/job1/rpu_edit.json"));
    let enc = shell.iter().find(|c| c.contains("x265")).unwrap();
    // Crop only: FFmpeg feeds x265 directly, no VapourSynth in the way.
    assert!(
        enc.contains("-map 0:0 -vf crop=3840:1608:0:276 -fps_mode passthrough -pix_fmt yuv420p10le -strict -1 -f yuv4mpegpipe - | x265 --input - --y4m --preset medium"),
        "{enc}"
    );
    // The fixture has no file size, so there is no size target: fixed CRF.
    assert!(enc.contains("--crf 18 "), "{enc}");
    assert!(plan
        .warnings
        .iter()
        .any(|w| w.starts_with("Size target skipped")));
    assert!(enc.contains("--output-depth 10 --profile main10"));
    assert!(enc.contains("--colorprim bt2020 --transfer smpte2084 --colormatrix bt2020nc --range limited --chromaloc 2"));
    assert!(enc.contains("--hdr10 --hdr10-opt --repeat-headers"));
    assert!(enc.contains(
        "--master-display 'G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(40000000,50)'"
    ));
    assert!(enc.contains("--max-cll 1105,307"));
    assert!(
        enc.contains("--dolby-vision-profile 8.1 --dolby-vision-rpu /work/job1/RPU_cropped.bin")
    );
    assert!(
        !enc.contains("--dhdr10-info"),
        "HDR10+ is injected after encoding, not by x265"
    );
    assert!(all.contains("hdr10plus_tool inject -i /work/job1/video.hevc -j /work/job1/hdr10plus.json -o /work/job1/video.hdr10plus.hevc"));
    let mux_line = shell.iter().find(|c| c.starts_with("mkvmerge")).unwrap();
    assert!(
        mux_line.contains("/work/job1/video.hdr10plus.hevc"),
        "mux uses the injected stream"
    );

    // Mux keeps lossless then AC-3, then subtitles.
    let mux = shell.iter().find(|c| c.starts_with("mkvmerge")).unwrap();
    assert!(mux.contains("--audio-tracks 1,2"));
    assert!(mux.contains("--track-order 0:0,1:1,1:2,1:4,1:5,1:6"));
    assert!(mux.contains("--default-track-flag 1:yes"));
    assert!(mux.contains("--default-track-flag 2:no"));

    assert!(plan.badges.contains(&"Dolby Vision 8.1".to_string()));
    assert!(plan.warnings.iter().any(|w| w.contains("profile 8.1")));

    // The script crops and outputs 10-bit.
    let script = &plan
        .steps
        .iter()
        .find(|s| s.kind == StepKind::WriteFile && s.file.as_ref().unwrap().0.ends_with("job.vpy"))
        .unwrap()
        .file
        .as_ref()
        .unwrap()
        .1;
    assert!(script.contains("core.std.Crop(clip, left=0, right=0, top=276, bottom=276)"));
}

#[test]
fn user_reorder_is_respected_in_mux() {
    let src = parse_ffprobe_json(&fixture("uhd_dovi_p7_truehd.json")).unwrap();
    let preset = PresetStore::builtin()
        .unwrap()
        .resolve(DEFAULT_PRESET, None)
        .unwrap();
    let mut audio = anvil_core::audio::default_audio_plan(&src.audio, &preset.audio);
    move_track(&mut audio, 1, 0).unwrap();
    let plan = plan_job(
        &src,
        &preset,
        &PlanOptions {
            audio_override: Some(audio),
            ..opts(None)
        },
    )
    .unwrap();
    let mux = plan
        .steps
        .iter()
        .find(|s| s.kind == StepKind::Mux)
        .unwrap()
        .command
        .as_ref()
        .unwrap()
        .to_shell();
    assert!(mux.contains("--track-order 0:0,1:2,1:1,"), "{mux}");
}

#[test]
fn av1_preset_drops_dolby_vision_with_warning() {
    let src = parse_ffprobe_json(&fixture("uhd_dovi_p7_truehd.json")).unwrap();
    let preset = PresetStore::builtin()
        .unwrap()
        .resolve("av1-archive", None)
        .unwrap();
    let plan = plan_job(&src, &preset, &opts(None)).unwrap();
    assert!(plan
        .warnings
        .iter()
        .any(|w| w.contains("Dolby Vision metadata will be dropped")));
    assert!(plan
        .warnings
        .iter()
        .any(|w| w.contains("HDR10+ metadata will be dropped")));
    let enc = plan
        .steps
        .iter()
        .find(|s| s.kind == StepKind::EncodeVideo)
        .unwrap()
        .command
        .as_ref()
        .unwrap()
        .to_shell();
    assert!(enc.contains("SvtAv1EncApp -i stdin --preset 4 --crf 26 --input-depth 10"));
    assert!(
        enc.contains("--color-primaries 9 --transfer-characteristics 16 --matrix-coefficients 9")
    );
    assert!(enc.contains("--content-light 1105,307"));
    assert!(!enc.contains("dolby"));
}

#[test]
fn video_passthrough_copies_video() {
    let src = parse_ffprobe_json(&fixture("uhd_dovi_p7_truehd.json")).unwrap();
    let job: toml::Value = toml::from_str("[output]\nvideo_passthrough = true").unwrap();
    let preset = PresetStore::builtin()
        .unwrap()
        .resolve(DEFAULT_PRESET, Some(&job))
        .unwrap();
    let plan = plan_job(
        &src,
        &preset,
        &opts(Some(Crop {
            top: 2,
            ..Default::default()
        })),
    )
    .unwrap();
    assert!(plan.encoder.is_none());
    assert!(plan.crop.is_none(), "passthrough never crops");
    assert!(plan.steps.iter().all(|s| s.kind != StepKind::EncodeVideo));
    let mux = plan
        .steps
        .iter()
        .find(|s| s.kind == StepKind::Mux)
        .unwrap()
        .command
        .as_ref()
        .unwrap()
        .to_shell();
    assert!(!mux.contains("--no-video"));
    assert!(mux.contains("--track-order 0:0,0:1,0:2,"));
}

#[test]
fn real_dvd_like_probe_plans_lpcm_then_ac3_and_deinterlaces() {
    let src = parse_ffprobe_json(&fixture("dvd_like_real_ffprobe.json")).unwrap();
    assert_eq!(src.video.resolution_class(), ResolutionClass::Sd);
    assert!(src.video.interlaced && src.video.top_field_first);
    assert_eq!(src.audio[0].codec, AudioCodec::Lpcm);

    let preset = PresetStore::builtin()
        .unwrap()
        .resolve(DEFAULT_PRESET, None)
        .unwrap();
    let plan = plan_job(&src, &preset, &opts(None)).unwrap();
    assert_eq!(plan.audio[0].source_codec, AudioCodec::Lpcm);
    assert_eq!(plan.audio[1].source_codec, AudioCodec::Ac3);
    assert!(plan.warnings.iter().any(|w| w.contains("BWDIF")));

    let script = plan.steps.iter().find_map(|s| s.file.as_ref()).unwrap();
    assert!(script.1.contains("core.bwdif.Bwdif(clip, field=1)"));
    let enc = plan
        .steps
        .iter()
        .find(|s| s.kind == StepKind::EncodeVideo)
        .unwrap()
        .command
        .as_ref()
        .unwrap()
        .to_shell();
    assert!(enc.contains("--crf 17"), "SD uses the SD CRF: {enc}");
}

#[test]
fn ac3_is_encoded_when_disc_has_none() {
    let mut src = parse_ffprobe_json(&fixture("uhd_dovi_p7_truehd.json")).unwrap();
    src.audio.retain(|a| a.codec != AudioCodec::Ac3);
    let preset = PresetStore::builtin()
        .unwrap()
        .resolve(DEFAULT_PRESET, None)
        .unwrap();
    let plan = plan_job(&src, &preset, &opts(None)).unwrap();
    assert_eq!(
        plan.audio[1].action,
        AudioAction::Encode {
            target: AudioTarget::Ac3,
            bitrate_kbps: 640,
            channels: 6
        }
    );
    let enc = plan
        .steps
        .iter()
        .find(|s| s.kind == StepKind::EncodeAudio)
        .unwrap()
        .command
        .as_ref()
        .unwrap()
        .to_shell();
    assert!(
        // aformat gives the encoder a plain 5.1 layout (TrueHD is 5.1(side)).
        enc.contains(
            "-map 0:a:0 -af aformat=channel_layouts=5.1 -c:a ac3 -b:a 640k /work/job1/audio_1.ac3"
        ),
        "{enc}"
    );
    let mux = plan
        .steps
        .iter()
        .find(|s| s.kind == StepKind::Mux)
        .unwrap()
        .command
        .as_ref()
        .unwrap()
        .to_shell();
    assert!(mux.contains("--track-order 0:0,1:1,2:0,"), "{mux}");
}

#[test]
fn exported_script_is_valid_bash_with_every_step() {
    let src = parse_ffprobe_json(&fixture("uhd_dovi_p7_truehd.json")).unwrap();
    let preset = PresetStore::builtin()
        .unwrap()
        .resolve(DEFAULT_PRESET, None)
        .unwrap();
    let plan = plan_job(
        &src,
        &preset,
        &opts(Some(Crop {
            top: 276,
            bottom: 276,
            ..Default::default()
        })),
    )
    .unwrap();
    let script = anvil_core::pipeline::to_shell_script(&plan);
    assert!(script.starts_with("#!/usr/bin/env bash"));
    assert!(script.contains("set -euo pipefail"));
    assert!(
        script.contains("mkdir -p /work/job1 '/out'")
            || script.contains("mkdir -p /work/job1 /out")
    );
    assert!(script.contains("cat > /work/job1/job.vpy <<'ANVIL_EOF'"));
    assert!(script.contains("clip.set_output()\nANVIL_EOF"));
    assert_eq!(script.matches("echo '==> [").count(), plan.steps.len());

    let path = std::env::temp_dir().join("anvil_export_test.sh");
    std::fs::write(&path, &script).unwrap();
    let ok = std::process::Command::new("bash")
        .arg("-n")
        .arg(&path)
        .status()
        .unwrap();
    assert!(ok.success(), "bash -n rejected the exported script");
}

#[test]
fn split_probe_passes_merge_frame_side_data() {
    // Header without frames, then a frame pass carrying HDR10+ and mastering data.
    let full: serde_json::Value =
        serde_json::from_str(&fixture("uhd_dovi_p7_truehd.json")).unwrap();
    let mut header = full.clone();
    header.as_object_mut().unwrap().remove("frames");
    let frames = serde_json::json!({ "frames": full["frames"] });
    let without = anvil_core::probe::parse_ffprobe_parts(&header.to_string(), None).unwrap();
    assert!(!without.video.hdr10_plus);
    let with =
        anvil_core::probe::parse_ffprobe_parts(&header.to_string(), Some(&frames.to_string()))
            .unwrap();
    assert!(with.video.hdr10_plus);
    assert_eq!(
        with.video.hdr10.unwrap().content_light.unwrap().max_cll,
        1105
    );
}

#[test]
fn plan_expectations_and_job_round_trip() {
    let src = parse_ffprobe_json(&fixture("uhd_dovi_p7_truehd.json")).unwrap();
    let preset = PresetStore::builtin()
        .unwrap()
        .resolve(DEFAULT_PRESET, None)
        .unwrap();
    let plan = plan_job(
        &src,
        &preset,
        &opts(Some(Crop {
            top: 276,
            bottom: 276,
            ..Default::default()
        })),
    )
    .unwrap();
    let e = &plan.expect;
    assert_eq!((e.width, e.height, e.bit_depth), (3840, 1608, 10));
    assert_eq!(e.video_codec, "hevc");
    assert!(e.mastering_display && e.content_light && e.hdr10_plus);
    assert_eq!(e.dolby_vision_profile, Some(8));
    assert_eq!(e.audio, vec![AudioCodec::TrueHd, AudioCodec::Ac3]);
    // 7932.448 s at 23.976 fps
    assert_eq!(plan.expected_frames, Some(190189));

    // Plans survive a TOML round trip (job files).
    let text = toml::to_string(&plan).unwrap();
    let back: anvil_core::pipeline::JobPlan = toml::from_str(&text).unwrap();
    assert_eq!(back.steps.len(), plan.steps.len());
    assert_eq!(back.steps[4].command, plan.steps[4].command);
    assert_eq!(back.expect, plan.expect);
}

#[test]
fn m2ts_track_ids_are_remapped_in_mux() {
    let mut src = parse_ffprobe_json(&fixture("uhd_dovi_p7_truehd.json")).unwrap();
    src.container = "mpegts".into();
    let preset = PresetStore::builtin()
        .unwrap()
        .resolve(DEFAULT_PRESET, None)
        .unwrap();
    // mkvmerge split the TrueHD core, so ffprobe index 2 (AC-3) is mkvmerge 3, etc.
    let map: anvil_core::trackmap::TrackIdMap =
        [(0, 0), (1, 1), (2, 3), (3, 4), (4, 5), (5, 6), (6, 7)]
            .into_iter()
            .collect();
    let plan = plan_job(
        &src,
        &preset,
        &PlanOptions {
            track_ids: Some(map),
            ..opts(None)
        },
    )
    .unwrap();
    let mux = plan
        .steps
        .iter()
        .find(|s| s.kind == StepKind::Mux)
        .unwrap()
        .command
        .as_ref()
        .unwrap()
        .to_shell();
    assert!(mux.contains("--audio-tracks 1,3"), "{mux}");
    assert!(mux.contains("--subtitle-tracks 5,6,7"), "{mux}");
    assert!(
        mux.contains("--track-order 0:0,1:1,1:3,1:5,1:6,1:7"),
        "{mux}"
    );
    assert!(!plan.warnings.iter().any(|w| w.contains("Track IDs assume")));
}

/// Captured from a real UHD file (x265 HDR10 + Dolby Vision 8.1 RPU +
/// HDR10+ injected with hdr10plus_tool, muxed by mkvmerge).
#[test]
fn real_uhd_probe_detects_hdr10_dolby_vision_and_hdr10plus() {
    let src = anvil_core::probe::parse_ffprobe_parts(
        &fixture("real_uhd_dv81_hdr10plus_header.json"),
        Some(&fixture("real_uhd_dv81_hdr10plus_frames.json")),
    )
    .unwrap();
    let v = &src.video;
    assert_eq!(v.dynamic_range, DynamicRange::Hdr10);
    assert_eq!(v.dolby_vision.as_ref().unwrap().profile, 8);
    assert!(v.hdr10_plus, "HDR10+ must be detected from the frame pass");
    let md = v.hdr10.unwrap().mastering_display.unwrap();
    assert_eq!(md.max_luminance, 1000.0);
    assert_eq!(v.hdr10.unwrap().content_light.unwrap().max_cll, 1000);
}

#[test]
fn autocrop_threshold_scales_with_bit_depth() {
    let src = parse_ffprobe_json(&fixture("uhd_dovi_p7_truehd.json")).unwrap();
    let cmds = anvil_core::pipeline::autocrop_commands(&src, 4);
    assert_eq!(cmds.len(), 4);
    let vf = cmds[0]
        .args
        .iter()
        .skip_while(|a| *a != "-vf")
        .nth(1)
        .unwrap();
    assert!(
        vf.starts_with("cropdetect=limit=0.094"),
        "absolute limits break on 10-bit: {vf}"
    );
}

/// Real M2TS made from the gate source: Dolby Vision only in the frames,
/// TrueHD with its AC-3 core listed as a separate stream (same PID).
#[test]
fn real_m2ts_detects_dv_from_frames_and_skips_embedded_core() {
    let src = anvil_core::probe::parse_ffprobe_parts(
        &fixture("real_m2ts_header.json"),
        Some(&fixture("real_m2ts_frames.json")),
    )
    .unwrap();
    assert_eq!(src.video.dolby_vision.as_ref().map(|d| d.profile), Some(8));
    assert!(src.video.hdr10_plus);
    assert_eq!(src.audio.len(), 3);
    assert!(src.audio[1].embedded_core, "AC-3 core sharing TrueHD's PID");
    assert!(!src.audio[2].embedded_core, "separate AC-3 track");

    let preset = PresetStore::builtin()
        .unwrap()
        .resolve(DEFAULT_PRESET, None)
        .unwrap();
    let plan = plan_job(&src, &preset, &opts(None)).unwrap();
    assert_eq!(
        plan.audio
            .iter()
            .map(|a| a.source_index)
            .collect::<Vec<_>>(),
        vec![1, 3]
    );
    // Metadata tools read the HEVC stream through an FFmpeg pipe.
    let dv = plan
        .steps
        .iter()
        .find(|s| s.label.contains("Dolby Vision RPU"))
        .unwrap()
        .command
        .as_ref()
        .unwrap()
        .to_shell();
    assert!(
        dv.contains("-bsf:v hevc_mp4toannexb -f hevc - | dovi_tool extract-rpu - -o"),
        "{dv}"
    );
}

#[test]
fn size_target_adds_a_quality_search_before_the_encode() {
    let mut src = parse_ffprobe_json(include_str!("fixtures/uhd_dovi_p7_truehd.json")).unwrap();
    src.size_bytes = Some(50_100_000_000);
    let preset = PresetStore::builtin()
        .unwrap()
        .resolve(DEFAULT_PRESET, None)
        .unwrap();
    let plan = plan_job(&src, &preset, &opts(None)).unwrap();

    let budget = plan.budget.as_ref().expect("budget");
    assert_eq!(budget.target_bytes, 16_533_000_000);
    assert_eq!(plan.expect.max_bytes, Some(16_533_000_000));
    assert!(
        plan.badges.contains(&"Max 16.5 GB".to_string()),
        "{:?}",
        plan.badges
    );

    let search_idx = plan
        .steps
        .iter()
        .position(|s| s.kind == StepKind::ChooseQuality)
        .unwrap();
    let search = plan.steps[search_idx].quality.as_ref().unwrap();
    assert_eq!(search.samples.len(), 8);
    assert_eq!((search.base_crf, search.max_crf), (18.0, 26.0));
    let sample = search.samples[0].to_shell();
    assert!(sample.contains("-frames:v 120"), "{sample}");
    assert!(sample.contains("--crf '{crf}'"), "{sample}");
    assert!(
        !sample.contains("--dolby-vision-rpu"),
        "samples skip the RPU: {sample}"
    );
    assert!(
        sample.contains("--vbv-maxrate 160000"),
        "but keep its rate limits: {sample}"
    );

    let enc = plan
        .steps
        .iter()
        .find(|s| s.kind == StepKind::EncodeVideo)
        .unwrap();
    assert!(enc.depends_on.contains(&search_idx));
    assert_eq!(
        enc.crf_from.as_deref(),
        Some(std::path::Path::new("/work/job1/crf.txt"))
    );
    assert!(enc
        .command
        .as_ref()
        .unwrap()
        .to_shell()
        .contains("--crf '{crf}'"));

    let script = anvil_core::pipeline::to_shell_script(&plan);
    assert!(script.contains("sample_kbps() {"));
    assert!(script.contains("CRF=$(cat /work/job1/crf.txt)"));
    assert!(script.contains("--crf \"$CRF\""));
    assert!(!script.contains("'{crf}'"));
}

#[test]
fn stream_bitrates_from_makemkv_statistics_tags() {
    // MakeMKV writes the Matroska statistics tags with a language suffix;
    // mkvmerge writes them bare. Exact bytes over duration wins over BPS.
    let json = r#"{
      "streams": [
        {"index": 0, "codec_type": "video", "codec_name": "hevc", "width": 3840, "height": 2160,
         "pix_fmt": "yuv420p10le", "r_frame_rate": "24000/1001", "avg_frame_rate": "24000/1001"},
        {"index": 1, "codec_type": "audio", "codec_name": "truehd", "profile": "Dolby TrueHD + Dolby Atmos",
         "channels": 8, "tags": {"language": "eng", "BPS-eng": "4700000",
         "DURATION-eng": "01:59:00.000000000", "NUMBER_OF_BYTES-eng": "4462500000"}},
        {"index": 2, "codec_type": "audio", "codec_name": "ac3", "channels": 6,
         "tags": {"language": "eng", "BPS": "640000"}},
        {"index": 3, "codec_type": "audio", "codec_name": "dts", "channels": 6, "tags": {"language": "fre"}},
        {"index": 4, "codec_type": "subtitle", "codec_name": "hdmv_pgs_subtitle",
         "tags": {"language": "eng", "BPS-eng": "35000"}}
      ],
      "format": {"filename": "a.mkv", "format_name": "matroska,webm", "duration": "7140.0", "size": "50100000000"}
    }"#;
    let src = parse_ffprobe_json(json).unwrap();
    assert_eq!(src.size_bytes, Some(50_100_000_000));
    // 4,462,500,000 bytes over 7140 s = 5000 kb/s (not the 4700 BPS tag).
    assert!((src.audio[0].kbps.unwrap() - 5000.0).abs() < 0.01);
    assert_eq!(src.audio[1].kbps, Some(640.0));
    assert_eq!(src.audio[2].kbps, None);
    assert_eq!(src.subtitles[0].kbps, Some(35.0));
}
