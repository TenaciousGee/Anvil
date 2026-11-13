// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Preview frames and queue thumbnails as PNG/JPEG files.
//!
//! Two kinds of frame:
//! - **Source**: FFmpeg seeks straight to a time. Fast on any file, so it
//!   drives scrubbing and the crop editor (the crop is drawn over it).
//! - **After filters**: exactly what the encoder will be given. Crop-only
//!   jobs encode straight from FFmpeg, so the frame comes from the same
//!   decode and crop ([`direct_frame_command`]), with no indexing wait. Jobs
//!   with real filters render the frame from the job's VapourSynth script
//!   with `vspipe`; the first request on a big file waits for the source
//!   filter to index it.
//!
//! HDR (PQ or HLG) frames are tone-mapped to SDR for display only, so they
//! look natural on an ordinary monitor. Encodes are never tone-mapped.

use crate::command::Command;
use crate::crop::Crop;
use crate::model::{DynamicRange, SourceInfo};

/// Width of preview frames; the UI scales them to fit.
pub const PREVIEW_WIDTH: u32 = 1280;
pub const THUMB_WIDTH: u32 = 320;

/// FFmpeg filter chain that turns a frame into a display-ready RGB image.
/// HDR frames are scaled down before tone mapping: the float conversion is
/// the slow part, and at preview size it costs a ninth of 4K's time with no
/// visible difference.
/// Colour parameters are set on the frames first because frames arriving
/// from `vspipe` (y4m) carry no colour metadata, and zscale refuses to
/// convert from "unspecified".
pub fn display_filter(range: DynamicRange, width: u32) -> String {
    let scale = format!("scale={width}:-2:flags=bicubic");
    match range {
        DynamicRange::Sdr => format!("{scale},format=rgb24"),
        DynamicRange::Hdr10 | DynamicRange::Hlg => {
            let trc = if range == DynamicRange::Hlg {
                "arib-std-b67"
            } else {
                "smpte2084"
            };
            format!(
                "{scale},setparams=color_primaries=bt2020:color_trc={trc}:colorspace=bt2020nc:range=tv,\
                 zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,\
                 zscale=t=bt709:m=bt709:r=tv,format=rgb24"
            )
        }
    }
}

/// Seconds into the source for a 0..1 position, avoiding the very end.
pub fn time_at(source: &SourceInfo, fraction: f64) -> f64 {
    let d = source.duration_secs.unwrap_or(0.0);
    (d * fraction.clamp(0.0, 1.0)).min((d - 1.0).max(0.0))
}

/// Frame number for a 0..1 position (for the after-filters preview).
pub fn frame_at(source: &SourceInfo, fraction: f64) -> u64 {
    let fps = source
        .video
        .avg_frame_rate
        .or(source.video.frame_rate)
        .map(|r| r.as_f64())
        .unwrap_or(24.0);
    (time_at(source, fraction) * fps).floor() as u64
}

/// A frame of the untouched source at `seconds`.
pub fn source_frame_command(source: &SourceInfo, seconds: f64, out_png: &str) -> Command {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
        // Keyframes only: Blu-ray transport streams seek mid-GOP, and the first
        // decoded frame there is a grey smear with no reference picture.
        .args([
            "-skip_frame",
            "nokey",
            "-ss",
            &format!("{seconds:.3}"),
            "-i",
            &source.path,
        ])
        .args([
            "-map",
            &format!("0:{}", source.video.index),
            "-frames:v",
            "1",
        ])
        .args([
            "-vf",
            &display_filter(source.video.dynamic_range, PREVIEW_WIDTH),
        ])
        .args(["-y", out_png])
}

/// The frame at `seconds` exactly as a crop-only job hands it to the
/// encoder: the same FFmpeg decode and crop, frame-accurate. It seeks two
/// seconds early and decodes into the frame, so M2TS (no index, mid-GOP
/// landing) still gives a clean picture.
pub fn direct_frame_command(
    source: &SourceInfo,
    seconds: f64,
    crop: &Crop,
    out_png: &str,
) -> Command {
    let lead = seconds.min(2.0);
    let mut vf = String::new();
    if !crop.is_none() {
        let (w, h) = crop
            .validate(source.video.width, source.video.height)
            .unwrap_or((source.video.width, source.video.height));
        vf = format!("crop={w}:{h}:{}:{},", crop.left, crop.top);
    }
    vf.push_str(&display_filter(source.video.dynamic_range, PREVIEW_WIDTH));
    Command::new("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
        .args(["-ss", &format!("{:.3}", seconds - lead), "-i", &source.path])
        .args(["-ss", &format!("{lead:.3}")])
        .args([
            "-map",
            &format!("0:{}", source.video.index),
            "-frames:v",
            "1",
        ])
        .args(["-vf", &vf, "-y", out_png])
}

/// Frame `frame` of a VapourSynth script, exactly as it will be encoded.
pub fn script_frame_command(
    script: &str,
    frame: u64,
    range: DynamicRange,
    out_png: &str,
) -> Command {
    let n = frame.to_string();
    Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "yuv4mpegpipe",
            "-i",
            "-",
        ])
        .args([
            "-frames:v",
            "1",
            "-vf",
            &display_filter(range, PREVIEW_WIDTH),
            "-y",
            out_png,
        ])
        .piped_from(
            Command::new("vspipe").args(["-c", "y4m", "--start", &n, "--end", &n, script, "-"]),
        )
}

/// A small JPEG for queue cards, taken a quarter of the way in.
pub fn thumbnail_command(source: &SourceInfo, out_jpg: &str) -> Command {
    let t = time_at(source, 0.25);
    Command::new("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
        .args([
            "-skip_frame",
            "nokey",
            "-ss",
            &format!("{t:.3}"),
            "-i",
            &source.path,
        ])
        .args([
            "-map",
            &format!("0:{}", source.video.index),
            "-frames:v",
            "1",
        ])
        .args([
            "-vf",
            &display_filter(source.video.dynamic_range, THUMB_WIDTH),
            "-q:v",
            "4",
        ])
        .args(["-y", out_jpg])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src() -> SourceInfo {
        crate::probe::parse_ffprobe_json(include_str!("../tests/fixtures/uhd_dovi_p7_truehd.json"))
            .unwrap()
    }

    #[test]
    fn hdr_frames_are_tone_mapped_sdr_are_not() {
        // Scaled to preview size first, then tone-mapped (much faster at 4K).
        assert!(display_filter(DynamicRange::Hdr10, 1280).starts_with(
            "scale=1280:-2:flags=bicubic,setparams=color_primaries=bt2020:color_trc=smpte2084"
        ));
        assert!(display_filter(DynamicRange::Hdr10, 1280).contains("tonemap=tonemap=hable"));
        assert!(display_filter(DynamicRange::Hlg, 1280).contains("color_trc=arib-std-b67"));
        assert!(!display_filter(DynamicRange::Sdr, 1280).contains("tonemap"));
    }

    #[test]
    fn direct_frames_match_the_encoder_input() {
        let s = src();
        let crop = Crop {
            top: 276,
            bottom: 276,
            ..Default::default()
        };
        let c = direct_frame_command(&s, 600.0, &crop, "/tmp/f.png").to_shell();
        // Seek two seconds early, decode into the frame, crop like the encode.
        assert!(c.contains("-ss 598.000 -i"), "{c}");
        assert!(c.contains("-ss 2.000 -map 0:0 -frames:v 1"), "{c}");
        assert!(c.contains("-vf crop=3840:1608:0:276,scale=1280"), "{c}");
        let c = direct_frame_command(&s, 0.5, &Crop::default(), "/tmp/f.png").to_shell();
        assert!(
            c.contains("-ss 0.000 -i") && c.contains("-ss 0.500 -map"),
            "{c}"
        );
        assert!(!c.contains("crop="), "{c}");
    }

    #[test]
    fn positions_and_commands() {
        let s = src();
        // 7932.448 s film: the end is clamped a second early.
        assert!((time_at(&s, 0.5) - 3966.224).abs() < 1e-6);
        assert!((time_at(&s, 1.0) - 7931.448).abs() < 1e-6);
        assert_eq!(frame_at(&s, 0.5), 95094);
        let c = source_frame_command(&s, 12.5, "/tmp/f.png").to_shell();
        assert!(c.contains(
            "-ss 12.500 -i '/rips/Example Film (2026)/title_t00.mkv' -map 0:0 -frames:v 1"
        ));
        let v =
            script_frame_command("/w/job.vpy", 240, DynamicRange::Hdr10, "/tmp/v.png").to_shell();
        assert!(
            v.starts_with("vspipe -c y4m --start 240 --end 240 /w/job.vpy - | ffmpeg"),
            "{v}"
        );
        assert!(thumbnail_command(&s, "/tmp/t.jpg")
            .to_shell()
            .contains("scale=320:-2"));
    }
}
