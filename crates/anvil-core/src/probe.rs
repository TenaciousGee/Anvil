// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Parse `ffprobe` JSON into a [`SourceInfo`].
//!
//! Probing is two quick passes:
//! 1. [`header_args`]: streams, format and chapters (container headers only);
//! 2. [`video_frame_args`]: the first decoded frame of the main video stream,
//!    where HEVC SEI carries HDR10 mastering data and HDR10+.
//!
//! Selecting the video stream in pass 2 matters: without it ffprobe returns
//! the first frame of *any* stream, often audio, and HDR side data is missed.
//! [`probe_file`] runs both with a timeout; the parsers stay pure so they are
//! tested against saved fixtures.

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::hdr::{parse_ratio, Chromaticity, ContentLightLevel, Hdr10Metadata, MasteringDisplay};
use crate::model::*;
use crate::process::run_with_timeout;
use crate::{Error, Result};

fn strs(args: &[&str], path: &str) -> Vec<String> {
    args.iter()
        .map(|s| s.to_string())
        .chain(std::iter::once(path.to_string()))
        .collect()
}

/// Pass 1: container-level information only. Fast even on 80 GB remuxes.
pub fn header_args(path: &str) -> Vec<String> {
    strs(
        &[
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            "-show_chapters",
        ],
        path,
    )
}

/// Pass 2: decode one frame of the first video stream for its side data.
pub fn video_frame_args(path: &str) -> Vec<String> {
    strs(
        &[
            "-v",
            "error",
            "-print_format",
            "json",
            "-select_streams",
            "v:0",
            "-read_intervals",
            "%+#1",
            // Full frame output: narrowing it with -show_entries strips each
            // side-data block's `side_data_type`, which hides HDR10+.
            "-show_frames",
        ],
        path,
    )
}

/// Single-call variant kept for the verify step and saved fixtures.
pub fn ffprobe_args(path: &str) -> Vec<String> {
    strs(
        &[
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            "-show_chapters",
        ],
        path,
    )
}

/// Run both probe passes with a timeout and parse the result.
///
/// The frame pass is best-effort: if it fails or times out (exotic codecs,
/// damaged first GOP) the source is still usable with stream-level metadata.
pub fn probe_file(path: &str, timeout: Duration) -> Result<SourceInfo> {
    let header = run_with_timeout("ffprobe", &header_args(path), timeout)?;
    if !header.success {
        return Err(Error::Tool {
            tool: "ffprobe".into(),
            message: first_line_or(&header.stderr, "could not read the file"),
        });
    }
    let frames = run_with_timeout("ffprobe", &video_frame_args(path), timeout)
        .ok()
        .filter(|o| o.success)
        .map(|o| o.stdout);
    let mut info = parse_ffprobe_parts(&header.stdout, frames.as_deref())?;
    if info.path.is_empty() {
        info.path = path.to_string();
    }
    Ok(info)
}

fn first_line_or(s: &str, fallback: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .unwrap_or_else(|| fallback.into())
}

/// Parse a header pass plus an optional frame pass.
pub fn parse_ffprobe_parts(header_json: &str, frames_json: Option<&str>) -> Result<SourceInfo> {
    let mut probe: Probe = serde_json::from_str(header_json)?;
    if let Some(f) = frames_json {
        if let Ok(extra) = serde_json::from_str::<FramesOnly>(f) {
            probe.frames.extend(extra.frames);
        }
    }
    from_probe(probe)
}

#[derive(Debug, Deserialize)]
struct FramesOnly {
    #[serde(default)]
    frames: Vec<Frame>,
}

#[derive(Debug, Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<Stream>,
    #[serde(default)]
    frames: Vec<Frame>,
    #[serde(default)]
    chapters: Vec<Value>,
    format: Option<Format>,
}

#[derive(Debug, Deserialize)]
struct Format {
    filename: Option<String>,
    format_name: Option<String>,
    duration: Option<String>,
    size: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Stream {
    index: u32,
    /// Container-level ID; the PID for MPEG-TS ("0x1011").
    id: Option<String>,
    codec_type: String,
    codec_name: String,
    profile: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    pix_fmt: Option<String>,
    bits_per_raw_sample: Option<String>,
    field_order: Option<String>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
    color_range: Option<String>,
    bit_rate: Option<String>,
    duration: Option<String>,
    color_space: Option<String>,
    color_transfer: Option<String>,
    color_primaries: Option<String>,
    chroma_location: Option<String>,
    channels: Option<u32>,
    tags: Tags,
    disposition: Disposition,
    side_data_list: Vec<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Tags {
    language: Option<String>,
    title: Option<String>,
    /// Everything else, including the Matroska statistics tags (mkvmerge
    /// writes them bare, MakeMKV with a language suffix: "BPS-eng").
    #[serde(flatten)]
    other: std::collections::BTreeMap<String, Value>,
}

impl Tags {
    /// A tag by name, ignoring case and any "-xxx" language suffix.
    fn get(&self, name: &str) -> Option<&str> {
        self.other.iter().find_map(|(k, v)| {
            let base = k.split('-').next().unwrap_or(k);
            (base.eq_ignore_ascii_case(name))
                .then(|| v.as_str())
                .flatten()
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Disposition {
    default: u8,
    forced: u8,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Frame {
    media_type: String,
    stream_index: Option<u32>,
    side_data_list: Vec<Value>,
}

/// Parse a single ffprobe JSON document (streams, and optionally frames).
pub fn parse_ffprobe_json(json: &str) -> Result<SourceInfo> {
    from_probe(serde_json::from_str(json)?)
}

fn from_probe(probe: Probe) -> Result<SourceInfo> {
    let video_stream = probe
        .streams
        .iter()
        .find(|s| s.codec_type == "video" && !is_cover_art(s))
        .ok_or(Error::NoVideo)?;

    // Side data can appear on the stream (Matroska, MP4) or on the first
    // frame (raw HEVC SEI), so gather both.
    let frame_side_data: Vec<&Value> = probe
        .frames
        .iter()
        .filter(|f| {
            f.media_type == "video" && f.stream_index.is_none_or(|i| i == video_stream.index)
        })
        .flat_map(|f| f.side_data_list.iter())
        .collect();
    let side_data: Vec<&Value> = video_stream
        .side_data_list
        .iter()
        .chain(frame_side_data.iter().copied())
        .collect();

    let video = build_video(video_stream, &side_data);

    let mut audio_ordinal = 0;
    let audio: Vec<AudioTrack> = probe
        .streams
        .iter()
        .filter(|s| s.codec_type == "audio")
        .map(|s| {
            let profile = s.profile.clone();
            let codec = AudioCodec::from_ffprobe(&s.codec_name, profile.as_deref());
            let lower = profile.as_deref().unwrap_or("").to_ascii_lowercase();
            let t = AudioTrack {
                index: s.index,
                pid: pid(s),
                audio_ordinal,
                codec,
                codec_name: s.codec_name.clone(),
                profile,
                channels: s.channels.unwrap_or(2),
                language: normalise_lang(s.tags.language.as_deref()),
                title: s.tags.title.clone(),
                object_audio: lower.contains("atmos") || lower.contains("dts:x"),
                embedded_core: false,
                default: s.disposition.default == 1,
                kbps: stream_kbps(s),
            };
            audio_ordinal += 1;
            t
        })
        .collect();

    // In M2TS, ffprobe exposes TrueHD's AC-3 core as an extra stream with the
    // same PID. mkvmerge cannot mux it separately, so flag it.
    let mut audio: Vec<AudioTrack> = audio;
    let lossless_pids: Vec<u32> = audio
        .iter()
        .filter(|a| a.codec == AudioCodec::TrueHd)
        .filter_map(|a| a.pid)
        .collect();
    for a in audio.iter_mut() {
        if a.codec == AudioCodec::Ac3 && a.pid.is_some_and(|p| lossless_pids.contains(&p)) {
            a.embedded_core = true;
        }
    }

    let subtitles = probe
        .streams
        .iter()
        .filter(|s| s.codec_type == "subtitle")
        .map(|s| SubtitleTrack {
            index: s.index,
            pid: pid(s),
            codec: SubtitleCodec::from_ffprobe(&s.codec_name),
            language: normalise_lang(s.tags.language.as_deref()),
            title: s.tags.title.clone(),
            forced: s.disposition.forced == 1,
            default: s.disposition.default == 1,
            kbps: stream_kbps(s),
        })
        .collect();

    let format = probe.format.as_ref();
    Ok(SourceInfo {
        path: format.and_then(|f| f.filename.clone()).unwrap_or_default(),
        container: format
            .and_then(|f| f.format_name.clone())
            .unwrap_or_default(),
        duration_secs: format
            .and_then(|f| f.duration.as_deref())
            .and_then(|d| d.parse().ok()),
        size_bytes: format
            .and_then(|f| f.size.as_deref())
            .and_then(|s| s.parse().ok()),
        video,
        audio,
        subtitles,
        chapter_count: probe.chapters.len(),
        chapters: crate::chapters::from_ffprobe(&probe.chapters),
    })
}

/// A stream's average bitrate in kb/s: exact bytes over duration from the
/// Matroska statistics tags when present, else the BPS tag or ffprobe's own
/// estimate.
fn stream_kbps(s: &Stream) -> Option<f64> {
    let t = &s.tags;
    let bytes = t
        .get("NUMBER_OF_BYTES")
        .and_then(|b| b.trim().parse::<f64>().ok());
    let secs = t.get("DURATION").and_then(parse_hms);
    if let (Some(b), Some(d)) = (bytes, secs) {
        if d > 0.0 {
            return Some(b * 8.0 / d / 1000.0);
        }
    }
    [t.get("BPS"), s.bit_rate.as_deref()]
        .into_iter()
        .flatten()
        .find_map(|v| v.trim().parse::<f64>().ok())
        .filter(|bps| *bps > 0.0)
        .map(|bps| bps / 1000.0)
}

/// "01:59:06.123456789" to seconds.
fn parse_hms(s: &str) -> Option<f64> {
    let mut parts = s.trim().split(':').rev();
    let secs: f64 = parts.next()?.parse().ok()?;
    let mins: f64 = parts.next().map_or(Some(0.0), |m| m.parse().ok())?;
    let hours: f64 = parts.next().map_or(Some(0.0), |h| h.parse().ok())?;
    Some(hours * 3600.0 + mins * 60.0 + secs)
}

fn pid(s: &Stream) -> Option<u32> {
    let id = s.id.as_deref()?;
    match id.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => id.parse().ok(),
    }
}

fn is_cover_art(s: &Stream) -> bool {
    matches!(s.codec_name.as_str(), "mjpeg" | "png" | "bmp")
}

fn normalise_lang(lang: Option<&str>) -> Option<String> {
    match lang {
        None | Some("") | Some("und") => None,
        Some(l) => Some(l.to_ascii_lowercase()),
    }
}

fn build_video(s: &Stream, side_data: &[&Value]) -> VideoTrack {
    let pix_fmt = s.pix_fmt.clone().unwrap_or_default();
    let bit_depth = s
        .bits_per_raw_sample
        .as_deref()
        .and_then(|b| b.parse().ok())
        .unwrap_or_else(|| bit_depth_from_pix_fmt(&pix_fmt));

    let frame_rate = s.r_frame_rate.as_deref().and_then(Rational::parse);
    let avg_frame_rate = s.avg_frame_rate.as_deref().and_then(Rational::parse);
    let vfr_suspected = match (frame_rate, avg_frame_rate) {
        (Some(r), Some(a)) => (r.as_f64() - a.as_f64()).abs() / r.as_f64() > 0.01,
        _ => false,
    };

    let interlaced = matches!(
        s.field_order.as_deref(),
        Some("tt") | Some("bb") | Some("tb") | Some("bt")
    );

    let dynamic_range = match s.color_transfer.as_deref() {
        Some("smpte2084") => DynamicRange::Hdr10,
        Some("arib-std-b67") => DynamicRange::Hlg,
        _ => DynamicRange::Sdr,
    };

    let mut hdr10 = Hdr10Metadata::default();
    let mut dolby_vision = None;
    let mut hdr10_plus = false;

    for sd in side_data {
        let kind = sd
            .get("side_data_type")
            .and_then(Value::as_str)
            .unwrap_or("");
        if kind.starts_with("Mastering display metadata") {
            hdr10.mastering_display = hdr10.mastering_display.or_else(|| mastering_display(sd));
        } else if kind.starts_with("Content light level metadata") {
            hdr10.content_light = hdr10.content_light.or_else(|| content_light(sd));
        } else if kind.starts_with("DOVI configuration record") {
            dolby_vision = dolby_vision.or_else(|| {
                let profile = sd.get("dv_profile")?.as_u64()? as u8;
                Some(DolbyVisionInfo {
                    profile,
                    level: sd.get("dv_level").and_then(Value::as_u64).map(|l| l as u8),
                    has_enhancement_layer: sd.get("el_present_flag").and_then(Value::as_u64)
                        == Some(1),
                })
            });
        } else if kind.contains("2094-40") || kind.contains("HDR10+") {
            hdr10_plus = true;
        } else if kind.starts_with("Dolby Vision Metadata") && dolby_vision.is_none() {
            // No container-level record (common in M2TS): derive the profile
            // from the RPU header, as dovi_tool does.
            dolby_vision = dv_from_rpu_header(sd);
        }
    }

    VideoTrack {
        duration_secs: s
            .duration
            .as_deref()
            .and_then(|d| d.parse::<f64>().ok())
            .or_else(|| s.tags.get("DURATION").and_then(parse_hms))
            .filter(|d| *d > 0.0),
        index: s.index,
        pid: pid(s),
        codec: s.codec_name.clone(),
        width: s.width.unwrap_or(0),
        height: s.height.unwrap_or(0),
        pix_fmt,
        bit_depth,
        frame_rate,
        avg_frame_rate,
        vfr_suspected,
        interlaced,
        top_field_first: matches!(s.field_order.as_deref(), Some("tt") | Some("tb")),
        colour: ColourInfo {
            primaries: s.color_primaries.clone(),
            transfer: s.color_transfer.clone(),
            matrix: s.color_space.clone(),
            range: s.color_range.clone(),
            chroma_location: s.chroma_location.clone(),
        },
        dynamic_range,
        hdr10: (!hdr10.is_empty()).then_some(hdr10),
        dolby_vision,
        hdr10_plus,
    }
}

fn dv_from_rpu_header(sd: &Value) -> Option<DolbyVisionInfo> {
    let rpu_profile = sd.get("vdr_rpu_profile")?.as_u64()?;
    let el_resampling = sd
        .get("el_spatial_resampling_filter_flag")
        .and_then(Value::as_u64)
        == Some(1);
    let profile = match rpu_profile {
        0 => 5,
        1 if el_resampling => 7,
        1 => 8,
        _ => return None,
    };
    Some(DolbyVisionInfo {
        profile,
        level: None,
        has_enhancement_layer: profile == 7,
    })
}

fn bit_depth_from_pix_fmt(pix_fmt: &str) -> u8 {
    if pix_fmt.contains("12") {
        12
    } else if pix_fmt.contains("10") {
        10
    } else {
        8
    }
}

fn ratio_field(v: &Value, key: &str) -> Option<f64> {
    match v.get(key)? {
        Value::String(s) => parse_ratio(s),
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

fn mastering_display(v: &Value) -> Option<MasteringDisplay> {
    let c = |x: &str, y: &str| -> Option<Chromaticity> {
        Some(Chromaticity {
            x: ratio_field(v, x)?,
            y: ratio_field(v, y)?,
        })
    };
    Some(MasteringDisplay {
        red: c("red_x", "red_y")?,
        green: c("green_x", "green_y")?,
        blue: c("blue_x", "blue_y")?,
        white_point: c("white_point_x", "white_point_y")?,
        max_luminance: ratio_field(v, "max_luminance")?,
        min_luminance: ratio_field(v, "min_luminance")?,
    })
}

fn content_light(v: &Value) -> Option<ContentLightLevel> {
    Some(ContentLightLevel {
        max_cll: v.get("max_content")?.as_u64()? as u32,
        max_fall: v.get("max_average")?.as_u64()? as u32,
    })
}
