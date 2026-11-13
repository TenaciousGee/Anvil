// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Dynamic HDR metadata after a crop: Dolby Vision L5 (active area) and
//! HDR10+ checks.
//!
//! L5 tells a Dolby Vision display where the picture is inside the frame:
//! offsets from each edge to the active image. A letterboxed film carries
//! the black bars there (top 280, bottom 280 for 2.40:1 in 3840x2160). Once
//! the bars are cropped away, the offsets must shrink by exactly what was
//! cut, frame by frame, and never go below zero. Zeroing every offset (what
//! `dovi_tool editor` does with `"crop": true`) is only right when the crop
//! removes all the bars in every scene; a partial crop, or a film whose
//! aspect ratio changes (IMAX sequences), needs the real arithmetic.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::crop::Crop;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offsets {
    pub left: u16,
    pub right: u16,
    pub top: u16,
    pub bottom: u16,
}

/// L5 metadata as `dovi_tool export -d level5=...` writes it: numbered
/// presets of offsets, and frame ranges ("0-1234") using each preset.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct L5 {
    pub presets: BTreeMap<u64, Offsets>,
    pub edits: Vec<(u64, u64, u64)>, // (first frame, last frame, preset id)
}

impl L5 {
    /// From a `level5` export (or an editor config with an active area).
    pub fn parse(json: &str) -> Result<L5, String> {
        let v: Value = serde_json::from_str(json).map_err(|e| format!("L5 JSON: {e}"))?;
        let area = v.get("active_area").unwrap_or(&v);
        let mut l5 = L5::default();
        for p in area
            .get("presets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let n = |k: &str| p.get(k).and_then(Value::as_u64).unwrap_or(0) as u16;
            let id = p
                .get("id")
                .and_then(Value::as_u64)
                .ok_or("L5 preset without an id")?;
            l5.presets.insert(
                id,
                Offsets {
                    left: n("left"),
                    right: n("right"),
                    top: n("top"),
                    bottom: n("bottom"),
                },
            );
        }
        for (range, id) in area
            .get("edits")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let (a, b) = range.split_once('-').unwrap_or((range, range));
            let (a, b) = (
                a.trim()
                    .parse()
                    .map_err(|_| format!("bad L5 range {range}"))?,
                b.trim()
                    .parse()
                    .map_err(|_| format!("bad L5 range {range}"))?,
            );
            l5.edits
                .push((a, b, id.as_u64().ok_or("bad L5 preset id")?));
        }
        l5.edits.sort();
        Ok(l5)
    }

    pub fn is_empty(&self) -> bool {
        self.presets.is_empty() || self.edits.is_empty()
    }

    /// Shrink every preset by the crop, never below zero.
    pub fn cropped(&self, crop: &Crop) -> L5 {
        let sub = |a: u16, b: u32| a.saturating_sub(b.min(u32::from(u16::MAX)) as u16);
        L5 {
            presets: self
                .presets
                .iter()
                .map(|(id, o)| {
                    (
                        *id,
                        Offsets {
                            left: sub(o.left, crop.left),
                            right: sub(o.right, crop.right),
                            top: sub(o.top, crop.top),
                            bottom: sub(o.bottom, crop.bottom),
                        },
                    )
                })
                .collect(),
            edits: self.edits.clone(),
        }
    }

    /// A `dovi_tool editor` config that writes exactly these offsets.
    pub fn editor_json(&self) -> String {
        let presets: Vec<Value> = self
            .presets
            .iter()
            .map(|(id, o)| json!({"id": id, "left": o.left, "right": o.right, "top": o.top, "bottom": o.bottom}))
            .collect();
        let edits: serde_json::Map<String, Value> = self
            .edits
            .iter()
            .map(|(a, b, id)| (format!("{a}-{b}"), json!(id)))
            .collect();
        serde_json::to_string_pretty(&json!({
            "active_area": {"crop": false, "presets": presets, "edits": edits}
        }))
        .unwrap_or_default()
    }

    /// Offsets for every frame.
    pub fn per_frame(&self) -> Vec<Offsets> {
        let last = self.edits.iter().map(|e| e.1).max().map_or(0, |m| m + 1);
        let zero = Offsets {
            left: 0,
            right: 0,
            top: 0,
            bottom: 0,
        };
        let mut v = vec![zero; last as usize];
        for (a, b, id) in &self.edits {
            let o = self.presets.get(id).copied().unwrap_or(zero);
            for f in *a..=*b {
                v[f as usize] = o;
            }
        }
        v
    }

    /// Distinct offsets, for messages: "top 0, bottom 0" or "2 shapes".
    pub fn describe(&self) -> String {
        let mut shapes: Vec<Offsets> = self.presets.values().copied().collect();
        shapes.dedup();
        match shapes.as_slice() {
            [o] => format!(
                "left {}, right {}, top {}, bottom {}",
                o.left, o.right, o.top, o.bottom
            ),
            _ => format!("{} active areas", shapes.len()),
        }
    }
}

/// Compare the output's L5 with what was expected, frame by frame.
pub fn compare_l5(expected: &L5, actual: &L5) -> Result<String, String> {
    let (e, a) = (expected.per_frame(), actual.per_frame());
    if e.len() != a.len() {
        return Err(format!(
            "L5 covers {} frames, expected {}",
            a.len(),
            e.len()
        ));
    }
    if let Some(f) = (0..e.len()).find(|&f| e[f] != a[f]) {
        return Err(format!(
            "frame {f}: found top {} bottom {} left {} right {}, expected top {} bottom {} left {} right {}",
            a[f].top, a[f].bottom, a[f].left, a[f].right, e[f].top, e[f].bottom, e[f].left, e[f].right
        ));
    }
    Ok(format!("{} frames, {}", e.len(), expected.describe()))
}

/// HDR10+ JSON from `hdr10plus_tool extract`: frames and the most
/// processing windows any frame uses.
pub fn hdr10plus_summary(json: &str) -> Result<(usize, u64), String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("HDR10+ JSON: {e}"))?;
    let frames = v
        .get("SceneInfo")
        .and_then(Value::as_array)
        .ok_or("HDR10+ JSON has no SceneInfo")?;
    let windows = frames
        .iter()
        .filter_map(|f| f.get("NumberOfWindows").and_then(Value::as_u64))
        .max()
        .unwrap_or(1);
    Ok((frames.len(), windows))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPORT: &str = r#"{"mode":0,"remove_cmv4":false,"remove_mapping":false,"active_area":{"crop":true,
        "presets":[{"id":0,"left":0,"right":0,"top":280,"bottom":280},{"id":1,"left":0,"right":0,"top":0,"bottom":0}],
        "edits":{"0-99":0,"100-149":1,"150-199":0}}}"#;

    fn crop(top: u32, bottom: u32) -> Crop {
        Crop {
            top,
            bottom,
            ..Default::default()
        }
    }

    #[test]
    fn crop_subtracts_per_scene() {
        let l5 = L5::parse(EXPORT).unwrap();
        assert_eq!(l5.edits, vec![(0, 99, 0), (100, 149, 1), (150, 199, 0)]);
        // A full crop of the bars: letterboxed scenes go to zero, the
        // full-frame (IMAX) scene stays zero.
        let full = l5.cropped(&crop(280, 280));
        assert_eq!(full.presets[&0].top, 0);
        assert_eq!(full.describe(), "left 0, right 0, top 0, bottom 0");
        // A partial crop leaves the rest of the bars described.
        let part = l5.cropped(&crop(200, 200));
        assert_eq!((part.presets[&0].top, part.presets[&0].bottom), (80, 80));
        assert_eq!(part.presets[&1].top, 0, "never below zero");
        assert_eq!(part.describe(), "2 active areas");

        // The editor config round-trips and writes explicit presets.
        let cfg = part.editor_json();
        assert!(cfg.contains("\"crop\": false"));
        assert_eq!(L5::parse(&cfg).unwrap(), part);
        assert_eq!(part.per_frame().len(), 200);
        assert_eq!(part.per_frame()[120].top, 0);
        assert_eq!(part.per_frame()[160].top, 80);
    }

    #[test]
    fn comparison() {
        let l5 = L5::parse(EXPORT).unwrap().cropped(&crop(280, 280));
        assert_eq!(
            compare_l5(&l5, &l5).unwrap(),
            "200 frames, left 0, right 0, top 0, bottom 0"
        );
        let wrong = L5::parse(EXPORT).unwrap();
        let e = compare_l5(&l5, &wrong).unwrap_err();
        assert!(e.starts_with("frame 0: found top 280"), "{e}");
        let mut short = l5.clone();
        short.edits.pop();
        assert!(compare_l5(&l5, &short).unwrap_err().contains("150 frames"));
        assert!(L5::parse("{}").unwrap().is_empty());
    }

    #[test]
    fn hdr10plus() {
        let j =
            r#"{"SceneInfo":[{"NumberOfWindows":1},{"NumberOfWindows":3}],"SceneInfoSummary":{}}"#;
        assert_eq!(hdr10plus_summary(j).unwrap(), (2, 3));
        assert!(hdr10plus_summary("{}").is_err());
    }
}
