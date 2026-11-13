// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Size target: keep the finished file under a share of the source size.
//!
//! The budget is worked out when the job is planned: the target size minus
//! the audio and subtitles that will be kept leaves a video bitrate. Just
//! before the encode, short sample encodes spread through the film measure
//! what the preset's CRF actually produces, and the CRF is raised only as far
//! as needed to fit (in the spirit of StaxRip's compressibility check). A
//! film that already fits keeps the preset's CRF, so quality comes first.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::audio::{AudioAction, AudioPlan};
use crate::command::Command;
use crate::model::{AudioCodec, SourceInfo, SubtitleCodec};
use crate::subtitles::{SubSource, SubtitlePlan};

/// Stands in for the CRF in commands until the runner has chosen it.
pub const CRF_PLACEHOLDER: &str = "{crf}";

/// Matroska headers, cues and block overhead, as a share of the file.
const CONTAINER_OVERHEAD: f64 = 0.005;

/// How far above the preset's CRF the search may go. Beyond this the
/// picture suffers more than a slightly larger file would.
pub const MAX_CRF_RAISE: f64 = 8.0;

/// Sample encodes: how many, and how long each one is.
pub const SAMPLE_COUNT: u32 = 8;
pub const SAMPLE_FRAMES: u64 = 120;

/// Two rounds of samples should cost at most this share of the encode;
/// shorter videos are encoded at the preset's CRF instead (about 22 minutes
/// at 24 fps).
pub const MAX_SAMPLE_SHARE: f64 = 0.06;

/// True when the video is long enough for sampling to be worth it.
pub fn worth_sampling(total_frames: f64) -> bool {
    2.0 * f64::from(SAMPLE_COUNT) * SAMPLE_FRAMES as f64 <= total_frames * MAX_SAMPLE_SHARE
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    pub percent: f64,
    pub source_bytes: u64,
    pub target_bytes: u64,
    pub duration_secs: f64,
    pub audio_kbps: f64,
    pub subtitle_kbps: f64,
    /// What is left for the video, in kb/s.
    pub video_kbps: f64,
    /// Tracks whose bitrate is a codec-based guess rather than measured.
    pub estimated: Vec<String>,
}

impl Budget {
    pub fn describe(&self) -> String {
        format!(
            "{} target ({}% of {}): video {:.1} Mb/s, audio {:.1} Mb/s",
            gb(self.target_bytes),
            fmt_pct(self.percent),
            gb(self.source_bytes),
            self.video_kbps / 1000.0,
            self.audio_kbps / 1000.0
        )
    }
}

pub fn gb(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{:.0} MB", bytes as f64 / 1e6)
    }
}

fn fmt_pct(p: f64) -> String {
    if p.fract() == 0.0 {
        format!("{p:.0}")
    } else {
        format!("{p:.1}")
    }
}

/// Work out the video bitrate that keeps the output at `percent` of the
/// source. `None` when the source size or duration is unknown.
pub fn budget(
    source: &SourceInfo,
    audio: &[AudioPlan],
    subtitles: &[SubtitlePlan],
    percent: f64,
) -> Option<Budget> {
    let source_bytes = source.size_bytes?;
    let duration = source.duration_secs.filter(|d| *d > 0.0)?;
    let target_bytes = (source_bytes as f64 * percent / 100.0) as u64;
    let mut estimated = Vec::new();

    let mut audio_kbps = 0.0;
    for a in audio {
        audio_kbps += match &a.action {
            AudioAction::Encode { bitrate_kbps, .. } => f64::from(*bitrate_kbps),
            AudioAction::Passthrough => {
                let track = source.audio.iter().find(|t| t.index == a.source_index);
                match track.and_then(|t| t.kbps) {
                    Some(k) => k,
                    None => {
                        let (channels, objects) =
                            track.map_or((6, false), |t| (t.channels, t.object_audio));
                        estimated.push(a.source_codec.display_name().to_string());
                        estimate_audio_kbps(a.source_codec, channels, objects)
                    }
                }
            }
        };
    }
    let subtitle_kbps: f64 = subtitles
        .iter()
        .map(|s| {
            let measured = match &s.source {
                // A handful of forced captions: next to nothing.
                SubSource::Track { .. } if s.forced_only => Some(2.0),
                SubSource::Track { index, .. } => source
                    .subtitles
                    .iter()
                    .find(|t| t.index == *index)
                    .and_then(|t| t.kbps),
                SubSource::File { path } => std::fs::metadata(path)
                    .ok()
                    .map(|m| m.len() as f64 * 8.0 / 1000.0 / duration),
            };
            measured.unwrap_or(match s.codec {
                // Blu-ray PGS: bitmaps, typically 20 to 60 kb/s.
                SubtitleCodec::Pgs => 40.0,
                SubtitleCodec::VobSub => 10.0,
                _ => 1.0,
            })
        })
        .sum();

    let total_kbps = target_bytes as f64 * 8.0 / 1000.0 / duration * (1.0 - CONTAINER_OVERHEAD);
    Some(Budget {
        percent,
        source_bytes,
        target_bytes,
        duration_secs: duration,
        audio_kbps,
        subtitle_kbps,
        video_kbps: total_kbps - audio_kbps - subtitle_kbps,
        estimated,
    })
}

/// Typical average bitrates, used when the container does not say. Erring
/// high keeps the output under target.
pub fn estimate_audio_kbps(codec: AudioCodec, channels: u32, objects: bool) -> f64 {
    let ch = f64::from(channels.max(1));
    match codec {
        AudioCodec::TrueHd if objects || channels >= 8 => 5500.0,
        AudioCodec::TrueHd => 3800.0,
        AudioCodec::DtsHdMa if channels >= 8 => 4800.0,
        AudioCodec::DtsHdMa => 3800.0,
        // 48 kHz, 24-bit.
        AudioCodec::Lpcm => ch * 1152.0,
        AudioCodec::Flac => ch * 600.0,
        AudioCodec::Dts => 1509.0,
        AudioCodec::DtsHdHra => 2000.0,
        AudioCodec::Ac3 => 640.0,
        AudioCodec::Eac3 => 1024.0,
        AudioCodec::Aac | AudioCodec::Mp2 => 320.0,
        AudioCodec::Opus => 256.0,
        AudioCodec::Other => 1000.0,
    }
}

/// Everything the runner needs to choose the quality value (x265's CRF,
/// NVEncC's QVBR quality, rav1e's quantizer...) before the encode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualitySearch {
    pub base_crf: f64,
    pub max_crf: f64,
    pub target_kbps: f64,
    /// How far the value moves to halve the bitrate (first guess only).
    #[serde(default = "default_per_halving")]
    pub per_halving: f64,
    /// Smallest step the encoder accepts (0.5 for x265 and x264, 1 for
    /// whole-number settings).
    #[serde(default = "default_step")]
    pub step: f64,
    /// What the value is called, for the log ("CRF", "QVBR quality"...).
    #[serde(default = "default_label")]
    pub label: String,
    /// Sample encodes. Their arguments contain [`CRF_PLACEHOLDER`].
    pub samples: Vec<Command>,
    /// The file each sample writes, in the same order.
    pub sample_outputs: Vec<String>,
    /// Playing time covered by all samples together.
    pub sample_secs: f64,
    /// Where the chosen CRF is written for the encode step.
    pub result_file: PathBuf,
}

fn default_per_halving() -> f64 {
    CRF_PER_HALVING
}
fn default_step() -> f64 {
    0.5
}
fn default_label() -> String {
    "CRF".into()
}

/// An encoder's constant-quality setting, as the size target sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Knob {
    /// The argument whose value is the quality ("--crf", "-qp").
    pub flag: &'static str,
    pub base: f64,
    pub max: f64,
    pub per_halving: f64,
    pub step: f64,
    pub label: &'static str,
}

/// The quality setting for each encoder, with how far the size target may
/// lower quality (about the same visual step for each: 8 x265 CRF).
pub fn knob(
    video: &crate::preset::VideoSection,
    kind: crate::encoder::EncoderKind,
    class: crate::model::ResolutionClass,
) -> Knob {
    use crate::encoder::EncoderKind as K;
    let k = |flag, base: f64, raise: f64, top: f64, per_halving, step, label| Knob {
        flag,
        base,
        max: (base + raise).min(top),
        per_halving,
        step,
        label,
    };
    match kind {
        K::X265 => k(
            "--crf",
            f64::from(video.x265.crf.pick(class)),
            MAX_CRF_RAISE,
            51.0,
            6.0,
            0.5,
            "CRF",
        ),
        K::X264 => k(
            "--crf",
            f64::from(video.x264.crf.pick(class)),
            MAX_CRF_RAISE,
            51.0,
            6.0,
            0.5,
            "CRF",
        ),
        // AV1's 0-63 scale moves a little further per halving.
        K::SvtAv1 => k(
            "--crf",
            f64::from(video.svt_av1.crf.pick(class)),
            12.0,
            63.0,
            8.0,
            1.0,
            "CRF",
        ),
        // rav1e's 0-255 quantizer: roughly four steps per x265 CRF.
        K::Rav1e => k(
            "--quantizer",
            f64::from(video.rav1e.quantizer.pick(class)),
            40.0,
            255.0,
            28.0,
            1.0,
            "quantizer",
        ),
        K::NvEncC => k(
            "--qvbr",
            f64::from(video.nvencc.quality.pick(class)),
            MAX_CRF_RAISE,
            51.0,
            6.0,
            1.0,
            "QVBR quality",
        ),
        K::QsvEncC => k(
            "--icq",
            f64::from(video.qsvencc.quality.pick(class)),
            MAX_CRF_RAISE,
            51.0,
            6.0,
            1.0,
            "ICQ",
        ),
        K::VceEncC => k(
            "--cqp",
            f64::from(video.vceencc.quality.pick(class)),
            MAX_CRF_RAISE,
            51.0,
            6.0,
            1.0,
            "QP",
        ),
        K::FfmpegVaapi => k(
            "-qp",
            f64::from(video.ffmpeg_vaapi.qp.pick(class)),
            MAX_CRF_RAISE,
            51.0,
            6.0,
            1.0,
            "QP",
        ),
    }
}

/// What to do after some sample rounds: `(crf, measured kb/s)` pairs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Next {
    Try(f64),
    Done(f64),
}

/// CRF steps of about 6 halve or double an x265 bitrate. Used for the first
/// guess; the second round measures the real slope for this film.
const CRF_PER_HALVING: f64 = 6.0;

pub fn decide(search: &QualitySearch, rounds: &[(f64, f64)]) -> Next {
    let (base, max, target) = (search.base_crf, search.max_crf, search.target_kbps);
    let (per_halving, step) = (search.per_halving, search.step.max(0.01));
    let clamp = |c: f64| round_up(c, step).clamp(base, max);
    match rounds {
        [] => Next::Try(base),
        [(c0, b0)] => {
            if *b0 <= target {
                return Next::Done(*c0);
            }
            let c1 = clamp(c0 + per_halving * (b0 / target).log2());
            if c1 <= *c0 {
                Next::Done(*c0)
            } else {
                Next::Try(c1)
            }
        }
        [.., (c0, b0), (c1, b1)] => {
            let slope = if b0 > &(b1 * 1.01) && c1 > c0 {
                ((c1 - c0) / (b0 / b1).log2()).clamp(per_halving / 3.0, per_halving * 2.0)
            } else {
                per_halving
            };
            Next::Done(clamp(c1 + slope * (b1 / target).log2()))
        }
    }
}

/// Round up to the next step (0.5 or 1): a touch smaller rather than a
/// touch over.
fn round_up(x: f64, step: f64) -> f64 {
    ((x / step) - 1e-9).ceil() * step
}

pub fn format_crf(c: f64) -> String {
    if c.fract() == 0.0 {
        format!("{c:.0}")
    } else {
        format!("{c:.1}")
    }
}

/// Replace the value after `--crf` (x265, SVT-AV1) with `value`.
pub fn with_crf(cmd: Command, value: &str) -> Command {
    with_quality(cmd, "--crf", value)
}

/// Replace the value after `flag` with `value`.
pub fn with_quality(mut cmd: Command, flag: &str, value: &str) -> Command {
    if let Some(i) = cmd.args.iter().position(|a| a == flag) {
        if let Some(v) = cmd.args.get_mut(i + 1) {
            *v = value.to_string();
        }
    }
    cmd
}

/// Start times of the sample encodes, spread over 5% to 95% of the film.
pub fn sample_starts(duration_secs: f64, count: u32) -> Vec<f64> {
    (0..count)
        .map(|i| duration_secs * (0.05 + 0.9 * (f64::from(i) + 0.5) / f64::from(count)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioRole;

    fn search(target: f64) -> QualitySearch {
        QualitySearch {
            per_halving: 6.0,
            step: 0.5,
            label: "CRF".into(),
            base_crf: 18.0,
            max_crf: 26.0,
            target_kbps: target,
            samples: vec![],
            sample_outputs: vec![],
            sample_secs: 40.0,
            result_file: PathBuf::from("crf.txt"),
        }
    }

    #[test]
    fn keeps_the_preset_crf_when_the_film_already_fits() {
        let s = search(15000.0);
        assert_eq!(decide(&s, &[]), Next::Try(18.0));
        assert_eq!(decide(&s, &[(18.0, 12000.0)]), Next::Done(18.0));
    }

    #[test]
    fn raises_crf_then_refines_with_the_measured_slope() {
        let s = search(12000.0);
        // Twice the target: about +6 CRF on the first guess.
        assert_eq!(decide(&s, &[(18.0, 24000.0)]), Next::Try(24.0));
        // This film is less sensitive: 6 CRF only took it to 15000 (slope
        // 6 / log2(1.6) = 8.85 per halving), so go further.
        match decide(&s, &[(18.0, 24000.0), (24.0, 15000.0)]) {
            Next::Done(c) => assert_eq!(c, 26.0, "clamped to the maximum"),
            other => panic!("{other:?}"),
        }
        // Overshot on the low side: come back down, but never below the preset.
        match decide(&s, &[(18.0, 14000.0), (20.0, 9000.0)]) {
            Next::Done(c) => assert!((18.0..20.0).contains(&c), "{c}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stops_at_the_cap() {
        let s = search(1000.0);
        assert_eq!(decide(&s, &[(18.0, 64000.0)]), Next::Try(26.0));
        let s = QualitySearch {
            per_halving: 6.0,
            step: 0.5,
            label: "CRF".into(),
            base_crf: 26.0,
            ..search(1000.0)
        };
        assert_eq!(decide(&s, &[(26.0, 64000.0)]), Next::Done(26.0));
    }

    #[test]
    fn crf_placeholder_replaces_the_value() {
        let c = Command::new("x265").args(["--preset", "medium", "--crf", "18", "--output", "o"]);
        assert_eq!(with_crf(c, CRF_PLACEHOLDER).args[3], "{crf}");
        assert_eq!(format_crf(20.5), "20.5");
        assert_eq!(format_crf(21.0), "21");
        assert_eq!(round_up(20.01, 0.5), 20.5);
        assert_eq!(round_up(20.0, 0.5), 20.0);
        assert_eq!(gb(16_533_000_000), "16.5 GB");
        assert_eq!(gb(5_200_000), "5 MB");
        // A 2-hour film is sampled; a 10-minute short is not.
        assert!(worth_sampling(7140.0 * 23.976));
        assert!(!worth_sampling(600.0 * 23.976));
    }

    #[test]
    fn budget_subtracts_audio_and_subtitles() {
        let mut src = crate::probe::parse_ffprobe_json(include_str!(
            "../tests/fixtures/uhd_dovi_p7_truehd.json"
        ))
        .unwrap();
        src.size_bytes = Some(50_100_000_000);
        src.duration_secs = Some(7140.0);
        let truehd = src.audio[0].clone();
        let plans = vec![
            AudioPlan {
                source_index: truehd.index,
                source_audio_ordinal: 0,
                source_codec: truehd.codec,
                action: AudioAction::Passthrough,
                role: AudioRole::Primary,
                language: None,
                title: None,
                default: true,
                delay_ms: 0,
                loudness_lufs: None,
            },
            AudioPlan {
                source_index: truehd.index,
                source_audio_ordinal: 0,
                source_codec: truehd.codec,
                action: AudioAction::Encode {
                    target: crate::audio::AudioTarget::Ac3,
                    bitrate_kbps: 640,
                    channels: 6,
                },
                role: AudioRole::Compatibility,
                language: None,
                title: None,
                default: false,
                delay_ms: 0,
                loudness_lufs: None,
            },
        ];
        let b = budget(&src, &plans, &[], 33.0).unwrap();
        assert_eq!(b.target_bytes, 16_533_000_000);
        // 16.533 GB over 7140 s is 18.52 Mb/s before overhead.
        let total = 16_533_000_000.0 * 8.0 / 1000.0 / 7140.0 * 0.995;
        assert!((b.video_kbps + b.audio_kbps - total).abs() < 0.01);
        // TrueHD with no statistics tags: estimated, and said so.
        assert_eq!(b.estimated, vec!["Dolby TrueHD".to_string()]);
        assert!(b.audio_kbps > 640.0 + 3000.0);
        assert!(b.describe().starts_with("16.5 GB target (33% of 50.1 GB)"));

        // Measured bitrate wins over the estimate.
        src.audio[0].kbps = Some(4200.0);
        let b = budget(&src, &plans, &[], 33.0).unwrap();
        assert!(b.estimated.is_empty());
        assert!((b.audio_kbps - 4840.0).abs() < 0.01);
    }

    #[test]
    fn knobs_for_every_encoder() {
        use crate::encoder::EncoderKind as K;
        use crate::model::ResolutionClass;
        let p = crate::preset::PresetStore::builtin()
            .unwrap()
            .resolve(crate::preset::DEFAULT_PRESET, None)
            .unwrap();
        let x = knob(&p.video, K::X265, ResolutionClass::Uhd);
        assert_eq!((x.flag, x.base, x.max, x.step), ("--crf", 18.0, 26.0, 0.5));
        let a = knob(&p.video, K::SvtAv1, ResolutionClass::Uhd);
        assert_eq!((a.flag, a.base, a.max, a.step), ("--crf", 26.0, 38.0, 1.0));
        let n = knob(&p.video, K::NvEncC, ResolutionClass::Uhd);
        assert_eq!(n.flag, "--qvbr");
        assert!(n.max <= 51.0);
        let r = knob(&p.video, K::Rav1e, ResolutionClass::Uhd);
        assert_eq!((r.flag, r.max), ("--quantizer", r.base + 40.0));
        assert_eq!(
            knob(&p.video, K::FfmpegVaapi, ResolutionClass::Hd).flag,
            "-qp"
        );

        // Whole-number settings move in whole steps.
        let q = QualitySearch {
            per_halving: 8.0,
            step: 1.0,
            label: "CRF".into(),
            base_crf: 26.0,
            max_crf: 38.0,
            target_kbps: 5000.0,
            samples: Vec::new(),
            sample_outputs: Vec::new(),
            sample_secs: 40.0,
            result_file: "q.txt".into(),
        };
        match decide(&q, &[(26.0, 10000.0)]) {
            Next::Try(c) => assert_eq!(c, 34.0),
            other => panic!("{other:?}"),
        }
        match decide(&q, &[(26.0, 10000.0), (34.0, 5600.0)]) {
            Next::Done(c) => assert!(c.fract() == 0.0 && c > 34.0 && c <= 38.0, "{c}"),
            other => panic!("{other:?}"),
        }
    }
}
