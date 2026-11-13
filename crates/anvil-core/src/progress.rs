// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Parse encoder progress lines into a common [`Progress`].
//!
//! Formats handled (all seen in real output):
//! - x265 fed by a pipe:  `49 frames: 103.93 fps, 452.09 kb/s`
//! - x265 with a total:   `[12.3%] 1234/10000 frames, 23.45 fps, 3456.78 kb/s, eta 0:06:12`
//! - NVEncC / QSVEncC / VCEEncC: `[12.3%] 1234 frames: 123.45 fps, 3456 kb/s, remain 0:01:23`
//! - SVT-AV1 (2.x):       `Encoding: 123/1000 Frames @ 12.34 fps | 3456.78 kbps | ...`
//! - SVT-AV1 (older):     `Encoding frame  123 2.34 kbps 12.34 fps`
//! - FFmpeg:              `size= 2346kB time=00:00:29.98 bitrate= 640.9kbits/s speed= 272x`
//!
//! Encoders reading a pipe cannot know the total, so the runner passes the
//! planner's expected frame count (or the source duration for FFmpeg).

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// 0.0 to 1.0 when the total is known.
    pub fraction: Option<f64>,
    pub frames: Option<u64>,
    pub total_frames: Option<u64>,
    pub fps: Option<f64>,
    pub kbps: Option<f64>,
    /// Seconds of media processed (FFmpeg).
    pub position_secs: Option<f64>,
    /// Realtime multiple (FFmpeg `speed=`).
    pub speed: Option<f64>,
    pub eta_secs: Option<u64>,
}

impl Progress {
    /// Compact one-line summary: "45.2% · 1234/2730 frames · 23.4 fps · ETA 6:12".
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(f) = self.fraction {
            parts.push(format!("{:.1}%", f * 100.0));
        }
        match (self.frames, self.total_frames) {
            (Some(n), Some(t)) => parts.push(format!("{n}/{t} frames")),
            (Some(n), None) => parts.push(format!("{n} frames")),
            _ => {}
        }
        if let Some(fps) = self.fps {
            parts.push(format!("{fps:.1} fps"));
        }
        if let Some(s) = self.speed {
            parts.push(format!("{s:.1}x"));
        }
        if let Some(eta) = self.eta_secs {
            parts.push(format!("ETA {}", format_duration(eta)));
        }
        parts.join(" · ")
    }
}

pub fn format_duration(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Which parser applies to a step's stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressKind {
    /// x265, SVT-AV1, rigaya encoders: frame counters.
    Frames,
    /// FFmpeg: `time=` against a known duration.
    Time,
    /// No progress output (dovi_tool, mkvmerge...).
    None,
}

impl ProgressKind {
    pub fn for_program(program: &str) -> Self {
        // Case-insensitive: rigaya's encoders are nvencc, qsvencc and
        // vceencc on Linux; without this NVEncC reported no progress at all.
        let p = program
            .rsplit('/')
            .next()
            .unwrap_or(program)
            .to_ascii_lowercase();
        match p.as_str() {
            "x265" | "x264" | "svtav1encapp" | "rav1e" | "nvencc" | "qsvencc" | "vceencc"
            | "nvencc64" | "qsvencc64" | "vceencc64" => ProgressKind::Frames,
            "ffmpeg" => ProgressKind::Time,
            _ => ProgressKind::None,
        }
    }
}

static FRAMES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d+)(?:/(\d+))?\s*[Ff]rames").unwrap());
static OLD_SVT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Encoding frame\s+(\d+)").unwrap());
static FPS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([\d.]+)\s*fps").unwrap());
static KBPS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)([\d.]+)\s*kb(?:/s|ps|its/s)").unwrap());
static TIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"time=\s*(\d+):(\d+):([\d.]+)").unwrap());
static SPEED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"speed=\s*([\d.]+)x").unwrap());

fn num<T: std::str::FromStr>(re: &Regex, line: &str, group: usize) -> Option<T> {
    re.captures(line)?.get(group)?.as_str().parse().ok()
}

/// Parse one stderr line. Returns `None` for lines that are not progress.
pub fn parse_line(
    kind: ProgressKind,
    line: &str,
    expected_frames: Option<u64>,
    duration_secs: Option<f64>,
) -> Option<Progress> {
    match kind {
        ProgressKind::Frames => parse_frames(line, expected_frames),
        ProgressKind::Time => parse_time(line, duration_secs),
        ProgressKind::None => None,
    }
}

fn parse_frames(line: &str, expected: Option<u64>) -> Option<Progress> {
    // Ignore x265's summary and info lines ("x265 [info]: frame I: ...").
    // x265's and x264's final "encoded 98 frames in 2.1s" is a summary;
    // rav1e's running "encoded 12 frames, 1.9 fps" is progress.
    if line.contains("[info]") || (line.contains("encoded ") && line.contains(" frames in ")) {
        return None;
    }
    let (frames, total) = if let Some(c) = FRAMES.captures(line) {
        (
            c.get(1)?.as_str().parse::<u64>().ok()?,
            c.get(2).and_then(|m| m.as_str().parse::<u64>().ok()),
        )
    } else {
        (num(&OLD_SVT, line, 1)?, None)
    };
    let fps: Option<f64> = num(&FPS, line, 1);
    let total = total.filter(|t| *t > 0).or(expected);
    let fraction = total.map(|t| (frames as f64 / t as f64).min(1.0));
    let eta_secs = match (total, fps) {
        (Some(t), Some(f)) if f > 0.0 && t >= frames => {
            Some(((t - frames) as f64 / f).round() as u64)
        }
        _ => None,
    };
    Some(Progress {
        fraction,
        frames: Some(frames),
        total_frames: total,
        fps,
        kbps: num(&KBPS, line, 1),
        eta_secs,
        ..Default::default()
    })
}

fn parse_time(line: &str, duration: Option<f64>) -> Option<Progress> {
    let c = TIME.captures(line)?;
    let h: f64 = c.get(1)?.as_str().parse().ok()?;
    let m: f64 = c.get(2)?.as_str().parse().ok()?;
    let s: f64 = c.get(3)?.as_str().parse().ok()?;
    let pos = h * 3600.0 + m * 60.0 + s;
    let speed: Option<f64> = num(&SPEED, line, 1);
    let fraction = duration.filter(|d| *d > 0.0).map(|d| (pos / d).min(1.0));
    let eta_secs = match (duration, speed) {
        (Some(d), Some(sp)) if sp > 0.0 && d >= pos => Some(((d - pos) / sp).round() as u64),
        _ => None,
    };
    Some(Progress {
        fraction,
        position_secs: Some(pos),
        speed,
        kbps: num(&KBPS, line, 1),
        eta_secs,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rav1e_and_x264_progress() {
        let p = parse_line(
            ProgressKind::Frames,
            "encoded 120 frames, 2.104 fps, 3456.78 Kb/s, est. size: 9.12 MB, est. time: 3m 12s",
            Some(518),
            None,
        )
        .unwrap();
        assert_eq!(p.frames, Some(120));
        assert_eq!(p.kbps, Some(3456.78));
        let x = parse_line(
            ProgressKind::Frames,
            "[44.1%] 120/272 frames, 31.20 fps, 1210.33 kb/s, eta 0:00:04",
            None,
            None,
        )
        .unwrap();
        assert_eq!((x.frames, x.total_frames), (Some(120), Some(272)));
        assert!(parse_line(
            ProgressKind::Frames,
            "encoded 272 frames in 8.71s (31.23 fps), 1210.33 kb/s, Avg QP:19.40",
            None,
            None
        )
        .is_none());
    }

    #[test]
    fn x265_piped_uses_expected_total() {
        let p = parse_line(
            ProgressKind::Frames,
            "49 frames: 103.93 fps, 452.09 kb/s  ",
            Some(98),
            None,
        )
        .unwrap();
        assert_eq!(p.frames, Some(49));
        assert_eq!(p.total_frames, Some(98));
        assert!((p.fraction.unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(p.fps, Some(103.93));
        assert_eq!(p.kbps, Some(452.09));
        assert_eq!(p.eta_secs, Some(0));
        assert!(parse_line(
            ProgressKind::Frames,
            "x265 [info]: frame I:      1, Avg QP:18.40",
            None,
            None
        )
        .is_none());
        assert!(parse_line(
            ProgressKind::Frames,
            "encoded 120 frames in 11.71s (10.25 fps)",
            None,
            None
        )
        .is_none());
    }

    #[test]
    fn x265_and_rigaya_with_totals() {
        let p = parse_line(
            ProgressKind::Frames,
            "[12.3%] 1230/10000 frames, 23.45 fps, 3456.78 kb/s, eta 0:06:12",
            None,
            None,
        )
        .unwrap();
        assert_eq!((p.frames, p.total_frames), (Some(1230), Some(10000)));
        assert_eq!(p.eta_secs, Some(374));
        let n = parse_line(
            ProgressKind::Frames,
            "[12.3%] 1234 frames: 123.45 fps, 3456 kb/s, remain 0:01:23",
            Some(10000),
            None,
        )
        .unwrap();
        assert_eq!(n.frames, Some(1234));
        assert_eq!(n.kbps, Some(3456.0));
    }

    #[test]
    fn svt_av1_formats() {
        let p = parse_line(
            ProgressKind::Frames,
            "Encoding: 123/1000 Frames @ 12.34 fps | 3456.78 kbps | Time: 0:00:10",
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            (p.frames, p.total_frames, p.fps),
            (Some(123), Some(1000), Some(12.34))
        );
        let old = parse_line(
            ProgressKind::Frames,
            "Encoding frame  123 2.34 kbps 12.34 fps",
            Some(1230),
            None,
        )
        .unwrap();
        assert_eq!(old.frames, Some(123));
        assert_eq!(old.fps, Some(12.34));
    }

    #[test]
    fn ffmpeg_time_and_speed() {
        let p = parse_line(
            ProgressKind::Time,
            "size=    2346kB time=00:00:15.00 bitrate= 640.9kbits/s speed= 30x    ",
            None,
            Some(30.0),
        )
        .unwrap();
        assert_eq!(p.position_secs, Some(15.0));
        assert_eq!(p.fraction, Some(0.5));
        assert_eq!(p.speed, Some(30.0));
        assert_eq!(p.eta_secs, Some(1));
        assert_eq!(p.kbps, Some(640.9));
        assert!(parse_line(
            ProgressKind::Time,
            "size=       0kB time=N/A bitrate=N/A speed=N/A",
            None,
            Some(30.0)
        )
        .is_none());
        assert_eq!(format_duration(374), "6:14");
        assert_eq!(format_duration(3725), "1:02:05");
    }

    #[test]
    fn rigaya_linux_names_and_progress() {
        for p in ["nvencc", "/usr/bin/nvencc", "NVEncC", "qsvencc", "vceencc"] {
            assert_eq!(ProgressKind::for_program(p), ProgressKind::Frames, "{p}");
        }
        let line = "[12.3%] 3383 frames: 410.53 fps, 12345 kb/s, remain 0:01:23, GPU 45%, VE 89%, VD 0%, est out size 1.2GB";
        let p = parse_line(ProgressKind::Frames, line, Some(27500), None).unwrap();
        assert_eq!(p.frames, Some(3383));
        assert_eq!(p.fps, Some(410.53));
        assert!(p.fraction.unwrap() > 0.12 && p.fraction.unwrap() < 0.13);
    }
}
