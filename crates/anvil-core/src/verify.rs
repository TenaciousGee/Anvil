// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Check a finished output against what the plan promised.
//!
//! The planner records [`Expectations`] (size, bit depth, HDR signalling,
//! Dolby Vision profile, HDR10+, audio order...). After muxing, the runner
//! probes the output with ffprobe and, when installed, MediaInfo, and every
//! promise becomes a [`Check`]. A failed critical check fails the job, so a
//! silently dropped HDR layer or audio track never goes unnoticed.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{AudioCodec, DynamicRange, SourceInfo};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expectations {
    pub duration_secs: Option<f64>,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    /// ffprobe codec name: "hevc", "av1", "h264"...
    pub video_codec: String,
    pub dynamic_range: DynamicRange,
    pub mastering_display: bool,
    pub content_light: bool,
    pub dolby_vision_profile: Option<u8>,
    pub hdr10_plus: bool,
    /// Output audio codecs in order.
    pub audio: Vec<AudioCodec>,
    pub subtitles: usize,
    pub chapters: bool,
    /// Exact number of chapters expected, when known.
    #[serde(default)]
    pub chapter_count: Option<usize>,
    /// Size target, when the preset sets one.
    #[serde(default)]
    pub max_bytes: Option<u64>,
    /// The Dolby Vision L5 the output must carry (written at encode time
    /// from the source's L5 less the crop).
    #[serde(default)]
    pub dv_l5: Option<PathBuf>,
    /// The source's HDR10+ metadata (for the processing-window check).
    #[serde(default)]
    pub hdr10plus_source: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    /// A failed critical check fails the job.
    pub critical: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VerifyReport {
    pub checks: Vec<Check>,
}

impl VerifyReport {
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.ok || !c.critical)
    }

    pub fn failures(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| !c.ok)
    }
}

pub(crate) fn check(name: &str, ok: bool, critical: bool, detail: String) -> Check {
    Check {
        name: name.into(),
        ok,
        critical,
        detail,
    }
}

/// Compare the probed output with the expectations.
pub fn verify(expect: &Expectations, out: &SourceInfo) -> VerifyReport {
    let v = &out.video;
    let mut c = Vec::new();

    c.push(check(
        "Video codec",
        v.codec == expect.video_codec,
        true,
        format!("expected {}, found {}", expect.video_codec, v.codec),
    ));
    c.push(check(
        "Picture size",
        (v.width, v.height) == (expect.width, expect.height),
        true,
        format!(
            "expected {}x{}, found {}x{}",
            expect.width, expect.height, v.width, v.height
        ),
    ));
    c.push(check(
        "Bit depth",
        v.bit_depth == expect.bit_depth,
        true,
        format!(
            "expected {}-bit, found {}-bit",
            expect.bit_depth, v.bit_depth
        ),
    ));
    if let (Some(max), Some(size)) = (expect.max_bytes, out.size_bytes) {
        // The CRF is chosen from samples, so allow a small overshoot before
        // flagging it. Not critical: the file is still a good encode.
        c.push(check(
            "Size target",
            size as f64 <= max as f64 * 1.03,
            false,
            format!(
                "{} of {} target ({:.0}%)",
                crate::sizing::gb(size),
                crate::sizing::gb(max),
                size as f64 / max as f64 * 100.0
            ),
        ));
    }
    if let (Some(want), Some(got)) = (expect.duration_secs, out.duration_secs) {
        // A frame or two of difference is normal at stream ends.
        let tolerance = (want * 0.01).max(1.0);
        c.push(check(
            "Duration",
            (want - got).abs() <= tolerance,
            true,
            format!("expected {want:.2}s, found {got:.2}s"),
        ));
    }
    c.push(check(
        "Dynamic range",
        v.dynamic_range == expect.dynamic_range,
        expect.dynamic_range != DynamicRange::Sdr,
        format!(
            "expected {:?}, found {:?}",
            expect.dynamic_range, v.dynamic_range
        ),
    ));
    if expect.mastering_display {
        let found = v.hdr10.and_then(|h| h.mastering_display);
        c.push(check(
            "HDR10 mastering display",
            found.is_some(),
            true,
            found
                .map(|m| format!("present, max {} nits", m.max_luminance))
                .unwrap_or_else(|| "missing".into()),
        ));
    }
    if expect.content_light {
        let found = v.hdr10.and_then(|h| h.content_light);
        c.push(check(
            "HDR10 MaxCLL / MaxFALL",
            found.is_some(),
            true,
            found
                .map(|l| format!("{} / {}", l.max_cll, l.max_fall))
                .unwrap_or_else(|| "missing".into()),
        ));
    }
    if let Some(profile) = expect.dolby_vision_profile {
        let found = v.dolby_vision.as_ref().map(|d| d.profile);
        c.push(check(
            "Dolby Vision",
            found == Some(profile),
            true,
            match found {
                Some(p) => format!("profile {p}"),
                None => format!("missing (expected profile {profile})"),
            },
        ));
    }
    if expect.hdr10_plus {
        c.push(check(
            "HDR10+",
            v.hdr10_plus,
            true,
            if v.hdr10_plus {
                "present".into()
            } else {
                "missing".into()
            },
        ));
    }

    let got_audio: Vec<AudioCodec> = out.audio.iter().map(|a| a.codec).collect();
    c.push(check(
        "Audio tracks and order",
        same_audio(&expect.audio, &got_audio),
        true,
        format!(
            "expected {}, found {}",
            names(&expect.audio),
            names(&got_audio)
        ),
    ));
    if let Some(first) = out.audio.first() {
        c.push(check(
            "Default audio track",
            first.default,
            false,
            if first.default {
                "first track is default".into()
            } else {
                "first track is not flagged default".into()
            },
        ));
    }
    c.push(check(
        "Subtitles",
        out.subtitles.len() == expect.subtitles,
        true,
        format!(
            "expected {}, found {}",
            expect.subtitles,
            out.subtitles.len()
        ),
    ));
    match expect.chapter_count {
        Some(n) => c.push(check(
            "Chapters",
            out.chapter_count == n,
            false,
            format!("expected {n}, found {}", out.chapter_count),
        )),
        None if expect.chapters => c.push(check(
            "Chapters",
            out.chapter_count > 0,
            false,
            format!("{} chapters", out.chapter_count),
        )),
        None => {}
    }
    VerifyReport { checks: c }
}

/// DTS-HD MA keeps its codec through passthrough, but some ffprobe builds
/// report the core profile on remux; treat any DTS family member as a match.
fn same_audio(want: &[AudioCodec], got: &[AudioCodec]) -> bool {
    let fam = |c: &AudioCodec| match c {
        AudioCodec::Dts | AudioCodec::DtsHdMa | AudioCodec::DtsHdHra => AudioCodec::Dts,
        other => *other,
    };
    want.len() == got.len() && want.iter().zip(got).all(|(a, b)| fam(a) == fam(b))
}

fn names(list: &[AudioCodec]) -> String {
    if list.is_empty() {
        return "none".into();
    }
    list.iter()
        .map(|c| c.display_name())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn mediainfo_args(path: &str) -> Vec<String> {
    vec!["--Output=JSON".into(), path.into()]
}

/// Independent HDR checks from MediaInfo's `HDR_Format`, which reads the
/// bitstream differently from FFmpeg. Only adds checks for promised features.
pub fn mediainfo_checks(expect: &Expectations, json: &str) -> Vec<Check> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return vec![check(
            "MediaInfo",
            false,
            false,
            "could not parse MediaInfo output".into(),
        )];
    };
    let video = root
        .pointer("/media/track")
        .and_then(Value::as_array)
        .and_then(|tracks| {
            tracks
                .iter()
                .find(|t| t.get("@type").and_then(Value::as_str) == Some("Video"))
        });
    let Some(video) = video else {
        return vec![check(
            "MediaInfo",
            false,
            true,
            "no video track found".into(),
        )];
    };
    let field = |k: &str| {
        video
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let hdr = format!(
        "{} {} {}",
        field("HDR_Format"),
        field("HDR_Format_Profile"),
        field("HDR_Format_Compatibility")
    );
    let mut c = Vec::new();

    let depth = field("BitDepth");
    c.push(check(
        "MediaInfo: bit depth",
        depth == expect.bit_depth.to_string(),
        true,
        format!("{} bits", if depth.is_empty() { "?" } else { &depth }),
    ));
    if expect.mastering_display {
        let ok = hdr.contains("2086") || !field("MasteringDisplay_Luminance").is_empty();
        c.push(check(
            "MediaInfo: HDR10 (SMPTE ST 2086)",
            ok,
            true,
            short(&hdr),
        ));
    }
    if let Some(p) = expect.dolby_vision_profile {
        let ok = hdr.contains("Dolby Vision")
            && (hdr.contains(&format!("Profile {p}")) || hdr.contains(&format!("dvhe.0{p}")));
        c.push(check("MediaInfo: Dolby Vision", ok, true, short(&hdr)));
    }
    if expect.hdr10_plus {
        let ok = hdr.contains("2094 App 4") || hdr.contains("HDR10+");
        c.push(check(
            "MediaInfo: HDR10+ (SMPTE ST 2094-40)",
            ok,
            true,
            short(&hdr),
        ));
    }
    c
}

fn short(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        "no HDR format reported".into()
    } else {
        s.chars().take(120).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expect() -> Expectations {
        Expectations {
            duration_secs: Some(10.0),
            width: 3840,
            height: 1608,
            bit_depth: 10,
            video_codec: "hevc".into(),
            dynamic_range: DynamicRange::Hdr10,
            mastering_display: true,
            content_light: true,
            dolby_vision_profile: Some(8),
            hdr10_plus: false,
            audio: vec![AudioCodec::TrueHd, AudioCodec::Ac3],
            subtitles: 1,
            chapters: false,
            chapter_count: None,
            dv_l5: None,
            hdr10plus_source: None,
            max_bytes: None,
        }
    }

    #[test]
    fn mediainfo_hdr_format_strings() {
        let json = r#"{"media":{"track":[{"@type":"General"},{"@type":"Video","BitDepth":"10",
            "HDR_Format":"Dolby Vision / SMPTE ST 2086","HDR_Format_Version":"1.0 / ",
            "HDR_Format_Profile":"dvhe.08 / ","HDR_Format_Compatibility":"HDR10 / HDR10"}]}}"#;
        let checks = mediainfo_checks(&expect(), json);
        assert!(checks.iter().all(|c| c.ok), "{checks:#?}");

        let sdr = r#"{"media":{"track":[{"@type":"Video","BitDepth":"8"}]}}"#;
        let checks = mediainfo_checks(&expect(), sdr);
        assert_eq!(checks.iter().filter(|c| !c.ok).count(), 3);
    }

    #[test]
    fn audio_family_matching() {
        assert!(same_audio(&[AudioCodec::DtsHdMa], &[AudioCodec::Dts]));
        assert!(!same_audio(
            &[AudioCodec::TrueHd, AudioCodec::Ac3],
            &[AudioCodec::Ac3, AudioCodec::TrueHd]
        ));
    }
}
