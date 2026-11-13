// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Crop values, validation and auto-crop aggregation.
//!
//! Auto-crop samples frames across the film (FFmpeg `cropdetect` or a
//! VapourSynth plugin) and keeps the *smallest* crop on each side, so films
//! with shifting aspect ratios (IMAX sequences) never lose picture.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crop {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
}

impl Crop {
    pub fn is_none(&self) -> bool {
        *self == Crop::default()
    }

    pub fn output_size(&self, width: u32, height: u32) -> Option<(u32, u32)> {
        let w = width.checked_sub(self.left + self.right)?;
        let h = height.checked_sub(self.top + self.bottom)?;
        (w > 0 && h > 0).then_some((w, h))
    }

    /// Check the crop against the source size. With 4:2:0 chroma every edge
    /// must be even.
    pub fn validate(&self, width: u32, height: u32) -> Result<(u32, u32)> {
        for (side, v) in [
            ("left", self.left),
            ("right", self.right),
            ("top", self.top),
            ("bottom", self.bottom),
        ] {
            if v % 2 != 0 {
                return Err(Error::Invalid {
                    field: "crop",
                    reason: format!("{side} crop of {v} px is odd; 4:2:0 video needs even values"),
                });
            }
        }
        self.output_size(width, height)
            .ok_or_else(|| Error::Invalid {
                field: "crop",
                reason: format!("crop {self:?} removes the whole {width}x{height} picture"),
            })
    }

    /// Round every side down to an even number (never crops more than detected).
    pub fn to_mod2(self) -> Crop {
        Crop {
            left: self.left & !1,
            right: self.right & !1,
            top: self.top & !1,
            bottom: self.bottom & !1,
        }
    }
}

/// Combine per-sample detections into one safe crop.
pub fn aggregate(samples: &[Crop]) -> Crop {
    if samples.is_empty() {
        return Crop::default();
    }
    let min = |f: fn(&Crop) -> u32| samples.iter().map(f).min().unwrap_or(0);
    Crop {
        left: min(|c| c.left),
        right: min(|c| c.right),
        top: min(|c| c.top),
        bottom: min(|c| c.bottom),
    }
    .to_mod2()
}

/// Parse an FFmpeg `cropdetect` log line (`... crop=1920:800:0:140`).
pub fn parse_cropdetect(line: &str, src_width: u32, src_height: u32) -> Option<Crop> {
    let spec = line
        .rsplit("crop=")
        .next()
        .filter(|_| line.contains("crop="))?;
    let spec = spec.split_whitespace().next()?;
    let mut parts = spec.split(':').map(|p| p.parse::<u32>());
    let w = parts.next()?.ok()?;
    let h = parts.next()?.ok()?;
    let x = parts.next()?.ok()?;
    let y = parts.next()?.ok()?;
    Some(Crop {
        left: x,
        top: y,
        right: src_width.checked_sub(w + x)?,
        bottom: src_height.checked_sub(h + y)?,
    })
}

/// Run FFmpeg auto-crop on a source: keyframes first, then every frame if
/// the source has too few keyframes for that to find anything. Calls
/// `on_sample(done, total)` after each sample. Returns the safe crop and how
/// many samples produced a detection (0 means nothing could be measured).
pub fn detect(
    source: &crate::model::SourceInfo,
    samples: u32,
    per_sample_timeout: std::time::Duration,
    mut on_sample: impl FnMut(usize, usize),
) -> (Crop, usize) {
    let (w, h) = (source.video.width, source.video.height);
    let run = |keyframes_only: bool, on_sample: &mut dyn FnMut(usize, usize)| -> Vec<Crop> {
        let cmds = crate::pipeline::autocrop_commands_with(source, samples, keyframes_only);
        let total = cmds.len();
        let mut found = Vec::new();
        for (i, cmd) in cmds.into_iter().enumerate() {
            if let Ok(out) =
                crate::process::run_with_timeout(&cmd.program, &cmd.args, per_sample_timeout)
            {
                if let Some(c) = out
                    .stderr
                    .lines()
                    .rev()
                    .find_map(|l| parse_cropdetect(l, w, h))
                {
                    found.push(c);
                }
            }
            on_sample(i + 1, total);
        }
        found
    };
    let mut found = run(true, &mut on_sample);
    if found.is_empty() {
        found = run(false, &mut on_sample);
    }
    (aggregate(&found), found.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cropdetect_line() {
        let line = "[Parsed_cropdetect_0 @ 0x55] x1:0 x2:1919 y1:140 y2:939 w:1920 h:800 x:0 y:140 pts:1 t:0.04 limit:0.094 crop=1920:800:0:140";
        let c = parse_cropdetect(line, 1920, 1080).unwrap();
        assert_eq!(
            c,
            Crop {
                left: 0,
                right: 0,
                top: 140,
                bottom: 140
            }
        );
        assert_eq!(c.validate(1920, 1080).unwrap(), (1920, 800));
        assert!(parse_cropdetect("no crop here", 1920, 1080).is_none());
    }

    #[test]
    fn aggregate_keeps_smallest_crop_for_imax_scenes() {
        let scope = Crop {
            left: 0,
            right: 0,
            top: 280,
            bottom: 280,
        };
        let imax = Crop {
            left: 0,
            right: 0,
            top: 0,
            bottom: 0,
        };
        assert_eq!(aggregate(&[scope, scope, imax]), Crop::default());
        assert_eq!(
            aggregate(&[scope, Crop { top: 279, ..scope }]),
            Crop { top: 278, ..scope }
        );
    }

    #[test]
    fn odd_crop_rejected() {
        let c = Crop {
            left: 1,
            ..Default::default()
        };
        assert!(c.validate(1920, 1080).is_err());
        let all = Crop {
            top: 540,
            bottom: 540,
            ..Default::default()
        };
        assert!(all.validate(1920, 1080).is_err());
    }
}
