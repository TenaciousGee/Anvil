// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Description of a source file as the engine sees it after probing.

use serde::{Deserialize, Serialize};

use crate::hdr::Hdr10Metadata;

/// Everything the planner needs to know about a source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceInfo {
    pub path: String,
    pub container: String,
    pub duration_secs: Option<f64>,
    /// Size of the source file in bytes (ffprobe `format.size`).
    #[serde(default)]
    pub size_bytes: Option<u64>,
    /// The source's chapters (times and titles).
    #[serde(default)]
    pub chapters: Vec<crate::chapters::Chapter>,
    pub video: VideoTrack,
    pub audio: Vec<AudioTrack>,
    pub subtitles: Vec<SubtitleTrack>,
    pub chapter_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoTrack {
    /// Stream index in the container (ffprobe `index`).
    pub index: u32,
    /// The video stream's own length, when the container records it. More
    /// reliable than the file's, which a long subtitle can stretch.
    #[serde(default)]
    pub duration_secs: Option<f64>,
    /// MPEG-TS packet ID (M2TS sources), used to match mkvmerge track IDs.
    #[serde(default)]
    pub pid: Option<u32>,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: String,
    pub bit_depth: u8,
    pub frame_rate: Option<Rational>,
    pub avg_frame_rate: Option<Rational>,
    /// True when the nominal and average frame rates disagree, which usually
    /// means variable frame rate. Confirmed later from container timestamps.
    pub vfr_suspected: bool,
    pub interlaced: bool,
    /// True when ffprobe reports top field first (`tt` or `tb`).
    pub top_field_first: bool,
    pub colour: ColourInfo,
    pub dynamic_range: DynamicRange,
    pub hdr10: Option<Hdr10Metadata>,
    pub dolby_vision: Option<DolbyVisionInfo>,
    pub hdr10_plus: bool,
}

impl VideoTrack {
    pub fn resolution_class(&self) -> ResolutionClass {
        ResolutionClass::from_dimensions(self.width, self.height)
    }

    pub fn is_hdr(&self) -> bool {
        !matches!(self.dynamic_range, DynamicRange::Sdr)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionClass {
    /// DVD and other standard-definition sources (height up to 576).
    Sd,
    /// 720p and 1080p.
    Hd,
    /// 4K / UHD and above.
    Uhd,
}

impl ResolutionClass {
    pub fn from_dimensions(width: u32, height: u32) -> Self {
        if width > 2048 || height > 1152 {
            ResolutionClass::Uhd
        } else if height > 576 || width > 1024 {
            ResolutionClass::Hd
        } else {
            ResolutionClass::Sd
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColourInfo {
    pub primaries: Option<String>,
    pub transfer: Option<String>,
    pub matrix: Option<String>,
    pub range: Option<String>,
    pub chroma_location: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicRange {
    Sdr,
    /// PQ (SMPTE ST 2084) transfer: HDR10 base layer.
    Hdr10,
    /// ARIB STD-B67 transfer.
    Hlg,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DolbyVisionInfo {
    pub profile: u8,
    pub level: Option<u8>,
    pub has_enhancement_layer: bool,
}

impl DolbyVisionInfo {
    /// Profile 7 carries a dual layer that cannot be re-encoded as-is; the
    /// planner converts its RPU to profile 8.1 for single-layer output.
    pub fn needs_profile_conversion(&self) -> bool {
        self.profile == 7
    }
}

/// A frame rate or other ratio as reported by ffprobe ("24000/1001").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rational {
    pub num: u32,
    pub den: u32,
}

impl Rational {
    pub fn parse(s: &str) -> Option<Self> {
        let (n, d) = s.split_once('/')?;
        let num: u32 = n.trim().parse().ok()?;
        let den: u32 = d.trim().parse().ok()?;
        if num == 0 || den == 0 {
            return None;
        }
        Some(Rational { num, den })
    }

    pub fn as_f64(&self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }
}

impl std::fmt::Display for Rational {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioTrack {
    /// Stream index in the container (ffprobe `index`).
    pub index: u32,
    #[serde(default)]
    pub pid: Option<u32>,
    /// Position among audio streams only (ffmpeg `0:a:N`).
    pub audio_ordinal: u32,
    pub codec: AudioCodec,
    /// Raw ffprobe codec name, kept for display and logging.
    pub codec_name: String,
    pub profile: Option<String>,
    pub channels: u32,
    pub language: Option<String>,
    pub title: Option<String>,
    /// Dolby Atmos (TrueHD or E-AC-3 JOC) or DTS:X object audio detected.
    pub object_audio: bool,
    /// An AC-3 core carried inside a TrueHD stream (same MPEG-TS PID).
    /// ffprobe lists it separately, but it cannot be muxed on its own.
    #[serde(default)]
    pub embedded_core: bool,
    pub default: bool,
    /// Average bitrate when the container reports it (Matroska statistics
    /// tags written by MakeMKV and mkvmerge, or ffprobe `bit_rate`).
    #[serde(default)]
    pub kbps: Option<f64>,
}

impl AudioTrack {
    pub fn label(&self) -> String {
        let mut s = format!(
            "{} {}",
            self.codec.display_name(),
            channel_layout(self.channels)
        );
        if self.object_audio {
            s.push_str(match self.codec {
                AudioCodec::DtsHdMa => " (DTS:X)",
                _ => " (Atmos)",
            });
        }
        if let Some(lang) = &self.language {
            s.push_str(&format!(" [{lang}]"));
        }
        if self.embedded_core {
            s.push_str(" (core inside TrueHD, not used)");
        }
        s
    }
}

pub fn channel_layout(channels: u32) -> String {
    match channels {
        1 => "1.0".into(),
        2 => "2.0".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        0 => "?".into(),
        n => format!("{n}ch"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    TrueHd,
    DtsHdMa,
    Lpcm,
    Flac,
    Dts,
    DtsHdHra,
    Ac3,
    Eac3,
    Aac,
    Opus,
    Mp2,
    Other,
}

impl AudioCodec {
    /// Classify from ffprobe's `codec_name` plus `profile`.
    pub fn from_ffprobe(codec_name: &str, profile: Option<&str>) -> Self {
        let profile = profile.unwrap_or("").to_ascii_lowercase();
        match codec_name {
            "truehd" | "mlp" => AudioCodec::TrueHd,
            "dts" => {
                if profile.contains("ma") {
                    AudioCodec::DtsHdMa
                } else if profile.contains("hra") || profile.contains("hd hra") {
                    AudioCodec::DtsHdHra
                } else {
                    AudioCodec::Dts
                }
            }
            c if c.starts_with("pcm_") => AudioCodec::Lpcm,
            "flac" => AudioCodec::Flac,
            "ac3" => AudioCodec::Ac3,
            "eac3" => AudioCodec::Eac3,
            "aac" => AudioCodec::Aac,
            "opus" => AudioCodec::Opus,
            "mp2" => AudioCodec::Mp2,
            _ => AudioCodec::Other,
        }
    }

    /// True for lossless formats.
    pub fn is_lossless(self) -> bool {
        matches!(
            self,
            AudioCodec::TrueHd | AudioCodec::DtsHdMa | AudioCodec::Lpcm | AudioCodec::Flac
        )
    }

    /// Preference among lossless formats when a disc carries more than one in
    /// the same language. Lower is preferred. Disc-native formats come first.
    pub fn lossless_rank(self) -> u8 {
        match self {
            AudioCodec::TrueHd => 0,
            AudioCodec::DtsHdMa => 1,
            AudioCodec::Lpcm => 2,
            AudioCodec::Flac => 3,
            _ => u8::MAX,
        }
    }

    /// Preference among lossy formats when no lossless track exists.
    pub fn lossy_rank(self) -> u8 {
        match self {
            AudioCodec::DtsHdHra => 0,
            AudioCodec::Eac3 => 1,
            AudioCodec::Dts => 2,
            AudioCodec::Ac3 => 3,
            AudioCodec::Aac => 4,
            AudioCodec::Opus => 5,
            AudioCodec::Mp2 => 6,
            _ => u8::MAX - 1,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            AudioCodec::TrueHd => "Dolby TrueHD",
            AudioCodec::DtsHdMa => "DTS-HD MA",
            AudioCodec::Lpcm => "LPCM",
            AudioCodec::Flac => "FLAC",
            AudioCodec::Dts => "DTS",
            AudioCodec::DtsHdHra => "DTS-HD HRA",
            AudioCodec::Ac3 => "Dolby Digital (AC-3)",
            AudioCodec::Eac3 => "Dolby Digital Plus (E-AC-3)",
            AudioCodec::Aac => "AAC",
            AudioCodec::Opus => "Opus",
            AudioCodec::Mp2 => "MPEG Audio",
            AudioCodec::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtitleTrack {
    pub index: u32,
    #[serde(default)]
    pub pid: Option<u32>,
    pub codec: SubtitleCodec,
    pub language: Option<String>,
    pub title: Option<String>,
    pub forced: bool,
    pub default: bool,
    #[serde(default)]
    pub kbps: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleCodec {
    Pgs,
    VobSub,
    Srt,
    Ass,
    Other,
}

impl SubtitleCodec {
    pub fn from_ffprobe(codec_name: &str) -> Self {
        match codec_name {
            "hdmv_pgs_subtitle" => SubtitleCodec::Pgs,
            "dvd_subtitle" => SubtitleCodec::VobSub,
            "subrip" | "srt" => SubtitleCodec::Srt,
            "ass" | "ssa" => SubtitleCodec::Ass,
            _ => SubtitleCodec::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_classes() {
        assert_eq!(
            ResolutionClass::from_dimensions(720, 480),
            ResolutionClass::Sd
        );
        assert_eq!(
            ResolutionClass::from_dimensions(720, 576),
            ResolutionClass::Sd
        );
        assert_eq!(
            ResolutionClass::from_dimensions(1280, 720),
            ResolutionClass::Hd
        );
        assert_eq!(
            ResolutionClass::from_dimensions(1920, 800),
            ResolutionClass::Hd
        );
        assert_eq!(
            ResolutionClass::from_dimensions(3840, 2160),
            ResolutionClass::Uhd
        );
        // Scope 4K crop still counts as UHD.
        assert_eq!(
            ResolutionClass::from_dimensions(3840, 1600),
            ResolutionClass::Uhd
        );
    }

    #[test]
    fn audio_codec_classification() {
        assert_eq!(
            AudioCodec::from_ffprobe("dts", Some("DTS-HD MA")),
            AudioCodec::DtsHdMa
        );
        assert_eq!(
            AudioCodec::from_ffprobe("dts", Some("DTS-HD MA + DTS:X")),
            AudioCodec::DtsHdMa
        );
        assert_eq!(
            AudioCodec::from_ffprobe("dts", Some("DTS")),
            AudioCodec::Dts
        );
        assert_eq!(AudioCodec::from_ffprobe("dts", None), AudioCodec::Dts);
        assert_eq!(
            AudioCodec::from_ffprobe("pcm_bluray", None),
            AudioCodec::Lpcm
        );
        assert_eq!(AudioCodec::from_ffprobe("truehd", None), AudioCodec::TrueHd);
        assert!(AudioCodec::TrueHd.is_lossless());
        assert!(!AudioCodec::Dts.is_lossless());
    }

    #[test]
    fn rational_parsing() {
        let r = Rational::parse("24000/1001").unwrap();
        assert!((r.as_f64() - 23.976).abs() < 0.001);
        assert_eq!(Rational::parse("0/0"), None);
    }
}
