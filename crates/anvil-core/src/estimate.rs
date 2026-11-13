// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! "How long will this take?": a few short timed encodes with the job's
//! own decode, filters and encoder settings, on this machine.
//!
//! At two places in the film the encode runs twice, for 24 and for 240
//! frames. The difference between the two times is the time for 216 frames
//! of steady encoding, so start-up (encoder set-up, seeking, the source
//! filter) does not distort the rate; that matters most for fast GPU
//! encoders, which finish 240 frames in about a second.

use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::command::Command;

/// Frames in the short and the long run at each place.
pub const RUN_FRAMES: [u64; 2] = [24, 240];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeedProbe {
    /// Pairs of (short, long) runs, place by place.
    pub samples: Vec<Command>,
    /// The file each run writes (deleted afterwards).
    pub outputs: Vec<String>,
    pub total_frames: u64,
    /// Frames the size target's sample encodes add (0 without a target).
    pub sizing_frames: u64,
    /// The VapourSynth script the runs read, written first (filtered jobs).
    pub script: Option<(PathBuf, String)>,
    pub encoder: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Estimate {
    /// Steady encoding speed, frames per second.
    pub fps: f64,
    /// The video encode.
    pub encode_secs: f64,
    /// Choosing the quality for the size target (0 without one).
    pub sizing_secs: f64,
}

impl Estimate {
    pub fn total_secs(&self) -> f64 {
        self.encode_secs + self.sizing_secs
    }

    pub fn describe(&self) -> String {
        let mut s = format!(
            "About {} on this machine ({:.1} fps",
            duration(self.total_secs()),
            self.fps
        );
        if self.sizing_secs > 0.0 {
            s.push_str(&format!(
                "; includes about {} choosing the quality for the size target",
                duration(self.sizing_secs)
            ));
        }
        s.push(')');
        s
    }
}

/// "2 h 05 min", "14 min", "under a minute".
pub fn duration(secs: f64) -> String {
    let m = (secs / 60.0).round() as u64;
    match m {
        0 => "under a minute".into(),
        1..=59 => format!("{m} min"),
        _ => format!("{} h {:02} min", m / 60, m % 60),
    }
}

fn spawn_pipeline(cmd: &Command) -> Result<Vec<Child>, String> {
    let start = |c: &Command, stdin: Stdio| {
        std::process::Command::new(&c.program)
            .args(&c.args)
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("{}: {e}", c.program))
    };
    match &cmd.stdin_from {
        Some(up) => {
            let mut u = start(up, Stdio::null())?;
            let out = u.stdout.take().ok_or("no output from the decoder")?;
            let mut d = match start(cmd, Stdio::from(out)) {
                Ok(d) => d,
                Err(e) => {
                    let _ = u.kill();
                    let _ = u.wait();
                    return Err(e);
                }
            };
            // The encoders' own stdout is not needed.
            drop(d.stdout.take());
            Ok(vec![u, d])
        }
        None => Ok(vec![start(cmd, Stdio::null())?]),
    }
}

/// Run one timed encode; `None` when cancelled.
fn timed(cmd: &Command, cancel: &AtomicBool) -> Result<Option<Duration>, String> {
    let t = Instant::now();
    let mut children = spawn_pipeline(cmd)?;
    let mut ok = true;
    loop {
        if cancel.load(Ordering::Relaxed) {
            for c in children.iter_mut() {
                let _ = c.kill();
                let _ = c.wait();
            }
            return Ok(None);
        }
        let mut all_done = true;
        for c in children.iter_mut() {
            match c.try_wait() {
                Ok(Some(s)) => ok &= s.success(),
                Ok(None) => all_done = false,
                Err(e) => return Err(e.to_string()),
            }
        }
        if all_done {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !ok {
        return Err(format!(
            "the test encode failed: {}",
            cmd.to_shell().chars().take(300).collect::<String>()
        ));
    }
    Ok(Some(t.elapsed()))
}

/// Time the probe's runs. `on_progress(done, total)` after each run.
pub fn run(
    probe: &SpeedProbe,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<Option<Estimate>, String> {
    if let Some((path, text)) = &probe.script {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, text).map_err(|e| e.to_string())?;
    }
    for out in &probe.outputs {
        if let Some(dir) = std::path::Path::new(out).parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
    }
    let n = probe.samples.len();
    let mut times = Vec::new();
    let result = (|| -> Result<Option<()>, String> {
        for (i, s) in probe.samples.iter().enumerate() {
            match timed(s, cancel)? {
                Some(t) => times.push(t.as_secs_f64()),
                None => return Ok(None),
            }
            on_progress(i + 1, n);
        }
        Ok(Some(()))
    })();
    for out in &probe.outputs {
        let _ = std::fs::remove_file(out);
    }
    if result?.is_none() {
        return Ok(None);
    }
    Ok(Some(from_times(probe, &times)))
}

/// The estimate from the measured (short, long) times.
pub fn from_times(probe: &SpeedProbe, times: &[f64]) -> Estimate {
    let extra = (RUN_FRAMES[1] - RUN_FRAMES[0]) as f64;
    let mut steady = 0.0;
    let mut pairs = 0.0;
    for pair in times.chunks(2) {
        if let [short, long] = pair {
            // A long run can never be faster than the short one; if timing
            // noise says so, fall back to the long run on its own.
            steady += if long > short {
                long - short
            } else {
                long * extra / RUN_FRAMES[1] as f64
            };
            pairs += 1.0;
        }
    }
    let fps = if steady > 0.0 {
        extra * pairs / steady
    } else {
        f64::INFINITY
    };
    Estimate {
        fps,
        encode_secs: probe.total_frames as f64 / fps,
        sizing_secs: probe.sizing_frames as f64 / fps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(total: u64, sizing: u64) -> SpeedProbe {
        SpeedProbe {
            samples: Vec::new(),
            outputs: Vec::new(),
            total_frames: total,
            sizing_frames: sizing,
            script: None,
            encoder: "x265".into(),
        }
    }

    #[test]
    fn startup_cancels_out() {
        // 3 s start-up, then 10 fps: 24 frames take 5.4 s, 240 take 27 s.
        let e = from_times(&probe(150_000, 0), &[5.4, 27.0, 5.4, 27.0]);
        assert!((e.fps - 10.0).abs() < 0.01, "{}", e.fps);
        assert_eq!(duration(e.total_secs()), "4 h 10 min");
        assert_eq!(e.describe(), "About 4 h 10 min on this machine (10.0 fps)");
        let s = from_times(&probe(150_000, 1440), &[5.4, 27.0, 5.4, 27.0]);
        assert!(s.describe().contains("2 min choosing the quality"));
        assert_eq!(duration(20.0), "under a minute");
    }

    #[test]
    fn runs_a_real_pipeline() {
        if crate::tools::which("ffmpeg").is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("anvil-est-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |frames: u64, name: &str| {
            let out = dir.join(name).to_string_lossy().to_string();
            Command::new("ffmpeg")
                .args([
                    "-v",
                    "error",
                    "-y",
                    "-f",
                    "yuv4mpegpipe",
                    "-i",
                    "-",
                    "-f",
                    "null",
                    &out,
                ])
                .piped_from(Command::new("ffmpeg").args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc2=size=320x180:rate=24",
                    "-frames:v",
                    &frames.to_string(),
                    "-f",
                    "yuv4mpegpipe",
                    "-",
                ]))
        };
        let p = SpeedProbe {
            samples: vec![mk(24, "a"), mk(240, "b")],
            outputs: vec![],
            total_frames: 24_000,
            sizing_frames: 0,
            script: None,
            encoder: "test".into(),
        };
        let mut seen = 0;
        let e = run(&p, &AtomicBool::new(false), |d, _| seen = d)
            .unwrap()
            .unwrap();
        assert_eq!(seen, 2);
        assert!(e.fps > 10.0, "{e:?}");
        let cancelled = run(&p, &AtomicBool::new(true), |_, _| {}).unwrap();
        assert!(cancelled.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
