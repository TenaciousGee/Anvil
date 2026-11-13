// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Subtitle tracks for the output: which to keep, their flags, files to
//! add, and forced-only tracks made from Blu-ray PGS.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::{SubtitleCodec, SubtitleTrack};
use crate::preset::SubtitleRules;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SubSource {
    /// A subtitle stream of the source: container index and its position
    /// among the source's subtitle streams (`0:s:N` in FFmpeg).
    Track { index: u32, ordinal: u32 },
    /// A file to add: SRT, ASS/SSA, PGS (.sup) or VobSub (.idx with .sub).
    File { path: String },
}

/// One output subtitle track, in output order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtitlePlan {
    pub source: SubSource,
    pub codec: SubtitleCodec,
    pub language: Option<String>,
    pub title: Option<String>,
    pub default: bool,
    pub forced: bool,
    /// Keep only the captions marked forced (PGS tracks only). The result is
    /// a new track built at encode time.
    #[serde(default)]
    pub forced_only: bool,
    /// Turn the images into text (SRT) with Tesseract. Image tracks only.
    #[serde(default)]
    pub ocr: bool,
}

impl SubtitlePlan {
    pub fn describe(&self) -> String {
        let what = match &self.source {
            SubSource::Track { .. } => codec_name(self.codec).to_string(),
            SubSource::File { path } => format!(
                "{} file {}",
                codec_name(self.codec),
                Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
        };
        let mut s = format!("{} {what}", self.language.as_deref().unwrap_or("und"));
        if let Some(t) = &self.title {
            s.push_str(&format!(" \"{t}\""));
        }
        if self.forced_only {
            s.push_str(", forced captions only");
        }
        if self.ocr {
            s.push_str(", to text (OCR)");
        }
        s
    }
}

pub fn codec_name(c: SubtitleCodec) -> &'static str {
    match c {
        SubtitleCodec::Pgs => "PGS",
        SubtitleCodec::VobSub => "VobSub",
        SubtitleCodec::Srt => "SRT",
        SubtitleCodec::Ass => "ASS",
        SubtitleCodec::Other => "subtitles",
    }
}

fn from_track(ordinal: usize, t: &SubtitleTrack) -> SubtitlePlan {
    SubtitlePlan {
        source: SubSource::Track {
            index: t.index,
            ordinal: ordinal as u32,
        },
        codec: t.codec,
        language: t.language.clone(),
        title: t.title.clone(),
        default: t.default,
        forced: t.forced,
        forced_only: false,
        ocr: false,
    }
}

/// The preset's rule: keep the listed languages (all when the list is
/// empty) plus forced tracks, with the source's flags.
pub fn default_subtitle_plan(tracks: &[SubtitleTrack], rules: &SubtitleRules) -> Vec<SubtitlePlan> {
    let mut plan: Vec<SubtitlePlan> = tracks
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            rules.languages.is_empty()
                || s.language
                    .as_ref()
                    .is_some_and(|l| rules.languages.contains(l))
                || (rules.keep_forced && s.forced)
        })
        .map(|(i, t)| from_track(i, t))
        .collect();
    single_default(&mut plan, None);
    plan
}

/// At most one default track: keep the first (or `keep`) and clear the rest.
fn single_default(plan: &mut [SubtitlePlan], keep: Option<usize>) {
    let first = keep.or_else(|| plan.iter().position(|p| p.default));
    for (i, p) in plan.iter_mut().enumerate() {
        p.default = Some(i) == first && p.default;
    }
}

/// What the editor sets on one output subtitle track.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SubEdit {
    pub language: Option<String>,
    pub title: Option<String>,
    pub default: bool,
    pub forced: bool,
    pub forced_only: bool,
    pub ocr: bool,
}

/// Can this track be turned into text? Bitmap subtitles only.
pub fn can_ocr(p: &SubtitlePlan) -> bool {
    matches!(p.codec, SubtitleCodec::Pgs | SubtitleCodec::VobSub)
}

pub fn edit_subtitle(plan: &mut [SubtitlePlan], index: usize, edit: &SubEdit) -> Result<()> {
    let p = plan.get_mut(index).ok_or(Error::Invalid {
        field: "subtitle track",
        reason: format!("no output subtitle track {}", index + 1),
    })?;
    if edit.forced_only
        && !(p.codec == SubtitleCodec::Pgs && matches!(p.source, SubSource::Track { .. }))
    {
        return Err(Error::Invalid {
            field: "forced only",
            reason: "forced captions can only be picked out of a Blu-ray (PGS) track".into(),
        });
    }
    if edit.ocr && !can_ocr(p) {
        return Err(Error::Invalid {
            field: "OCR",
            reason: "only image subtitles (PGS or VobSub) can be turned into text".into(),
        });
    }
    if let Some(l) = &edit.language {
        p.language = Some(normalise_language(l)?);
    }
    if let Some(t) = &edit.title {
        p.title = (!t.trim().is_empty()).then(|| t.trim().to_string());
    }
    p.forced_only = edit.forced_only;
    p.ocr = edit.ocr;
    // A forced-only track is, by definition, forced.
    p.forced = edit.forced || edit.forced_only;
    p.default = edit.default;
    if edit.default {
        single_default(plan, Some(index));
    }
    Ok(())
}

/// "eng", "en" or "EN" to what mkvmerge accepts (ISO 639-1 or -2).
pub fn normalise_language(l: &str) -> Result<String> {
    let l = l.trim().to_ascii_lowercase();
    if (2..=3).contains(&l.len()) && l.chars().all(|c| c.is_ascii_lowercase()) {
        Ok(l)
    } else {
        Err(Error::Invalid {
            field: "language",
            reason: format!("'{l}' is not a language code such as eng or en"),
        })
    }
}

pub fn add_source_subtitle(
    plan: &mut Vec<SubtitlePlan>,
    tracks: &[SubtitleTrack],
    ordinal: usize,
) -> Result<()> {
    let t = tracks.get(ordinal).ok_or(Error::Invalid {
        field: "subtitle track",
        reason: format!("the source has {} subtitle tracks", tracks.len()),
    })?;
    let mut p = from_track(ordinal, t);
    p.default = false;
    plan.push(p);
    Ok(())
}

/// Add a subtitle file. The language comes from the file name when it ends
/// in one ("Film.en.srt", "Film.eng.forced.srt").
pub fn add_file(plan: &mut Vec<SubtitlePlan>, path: &Path) -> Result<()> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let codec = match ext.as_str() {
        "srt" => SubtitleCodec::Srt,
        "ass" | "ssa" => SubtitleCodec::Ass,
        "sup" => SubtitleCodec::Pgs,
        "idx" => {
            if !path.with_extension("sub").is_file() {
                return Err(Error::Invalid {
                    field: "subtitle file",
                    reason: format!("{} needs its .sub file beside it", path.display()),
                });
            }
            SubtitleCodec::VobSub
        }
        _ => {
            return Err(Error::Invalid {
                field: "subtitle file",
                reason: format!(
                    "{} is not a subtitle file Anvil can add (.srt, .ass, .ssa, .sup, .idx)",
                    path.display()
                ),
            })
        }
    };
    if !path.is_file() {
        return Err(Error::Invalid {
            field: "subtitle file",
            reason: format!("{} not found", path.display()),
        });
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let parts: Vec<&str> = stem.split('.').collect();
    let forced = parts.iter().skip(1).any(|p| *p == "forced");
    let language = parts
        .iter()
        .skip(1)
        .rev()
        .find(|p| {
            (2..=3).contains(&p.len()) && p.chars().all(|c| c.is_ascii_lowercase()) && **p != "sdh"
        })
        .map(|p| p.to_string());
    plan.push(SubtitlePlan {
        source: SubSource::File {
            path: path.to_string_lossy().into_owned(),
        },
        codec,
        language,
        title: None,
        default: false,
        forced,
        forced_only: false,
        ocr: false,
    });
    Ok(())
}

pub fn move_subtitle(plan: &mut Vec<SubtitlePlan>, from: usize, to: usize) -> Result<()> {
    if from >= plan.len() || to >= plan.len() {
        return Err(Error::Invalid {
            field: "subtitle position",
            reason: format!("{from} -> {to} is out of range for {} tracks", plan.len()),
        });
    }
    let p = plan.remove(from);
    plan.insert(to, p);
    Ok(())
}

pub fn remove_subtitle(plan: &mut Vec<SubtitlePlan>, index: usize) -> Result<()> {
    if index >= plan.len() {
        return Err(Error::Invalid {
            field: "subtitle track",
            reason: format!("no output subtitle track {}", index + 1),
        });
    }
    plan.remove(index);
    Ok(())
}

/// Build one output track from a command-line spec:
/// `N[,forced][,forced-only][,default][,lang=XX][,title=TEXT]` for source
/// track N (1 = first), or `FILE[,...]` for a subtitle file.
pub fn parse_sub_spec(
    spec: &str,
    tracks: &[SubtitleTrack],
    plan: &mut Vec<SubtitlePlan>,
) -> Result<()> {
    let mut parts = spec.split(',');
    let head = parts.next().unwrap_or_default().trim();
    match head.parse::<usize>() {
        Ok(n) if n >= 1 => add_source_subtitle(plan, tracks, n - 1)?,
        _ => add_file(plan, Path::new(head))?,
    }
    let i = plan.len() - 1;
    let mut edit = SubEdit {
        language: None,
        title: None,
        default: false,
        forced: plan[i].forced,
        forced_only: false,
        ocr: false,
    };
    for opt in parts.map(str::trim) {
        match opt {
            "forced" => edit.forced = true,
            "forced-only" => edit.forced_only = true,
            "ocr" => edit.ocr = true,
            "default" => edit.default = true,
            _ => {
                if let Some(l) = opt.strip_prefix("lang=") {
                    edit.language = Some(l.to_string());
                } else if let Some(t) = opt.strip_prefix("title=") {
                    edit.title = Some(t.to_string());
                } else {
                    return Err(Error::Invalid {
                        field: "--sub",
                        reason: format!(
                            "unknown option '{opt}' (forced, forced-only, ocr, default, lang=XX, title=TEXT)"
                        ),
                    });
                }
            }
        }
    }
    edit_subtitle(plan, i, &edit)
}

/// Forced-only filtering of Blu-ray PGS streams.
pub mod pgs {
    use std::collections::HashMap;

    pub(crate) const PDS: u8 = 0x14;
    pub(crate) const ODS: u8 = 0x15;
    pub(crate) const PCS: u8 = 0x16;
    const WDS: u8 = 0x17;
    pub(crate) const END: u8 = 0x80;
    pub(crate) const FORCED: u8 = 0x40;
    const CROPPED: u8 = 0x80;
    const EPOCH_START: u8 = 0x80;

    #[derive(Debug, Clone)]
    pub(crate) struct Segment {
        pub(crate) pts: u32,
        pub(crate) dts: u32,
        pub(crate) kind: u8,
        pub(crate) payload: Vec<u8>,
    }

    impl Segment {
        fn write(&self, out: &mut Vec<u8>) {
            out.extend_from_slice(b"PG");
            out.extend_from_slice(&self.pts.to_be_bytes());
            out.extend_from_slice(&self.dts.to_be_bytes());
            out.push(self.kind);
            out.extend_from_slice(&(self.payload.len() as u16).to_be_bytes());
            out.extend_from_slice(&self.payload);
        }
    }

    pub(crate) fn segments(data: &[u8]) -> Result<Vec<Segment>, String> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 13 <= data.len() {
            if &data[i..i + 2] != b"PG" {
                return Err(format!("not a PGS stream (bad segment header at byte {i})"));
            }
            let pts = u32::from_be_bytes(data[i + 2..i + 6].try_into().unwrap());
            let dts = u32::from_be_bytes(data[i + 6..i + 10].try_into().unwrap());
            let kind = data[i + 10];
            let len = u16::from_be_bytes([data[i + 11], data[i + 12]]) as usize;
            let end = i + 13 + len;
            if end > data.len() {
                return Err("PGS stream ends mid-segment".into());
            }
            out.push(Segment {
                pts,
                dts,
                kind,
                payload: data[i + 13..end].to_vec(),
            });
            i = end;
        }
        Ok(out)
    }

    pub(crate) struct Object {
        pub(crate) id: u16,
        pub(crate) window: u8,
        pub(crate) flags: u8,
        pub(crate) rest: Vec<u8>, // x, y and the crop rectangle when cropped
    }

    /// Composition objects of a PCS payload.
    pub(crate) fn objects(pcs: &[u8]) -> Vec<Object> {
        let mut v = Vec::new();
        if pcs.len() < 11 {
            return v;
        }
        let n = pcs[10] as usize;
        let mut i = 11;
        for _ in 0..n {
            if i + 8 > pcs.len() {
                break;
            }
            let flags = pcs[i + 3];
            let len = if flags & CROPPED != 0 { 16 } else { 8 };
            if i + len > pcs.len() {
                break;
            }
            v.push(Object {
                id: u16::from_be_bytes([pcs[i], pcs[i + 1]]),
                window: pcs[i + 2],
                flags,
                rest: pcs[i + 4..i + len].to_vec(),
            });
            i += len;
        }
        v
    }

    fn rebuild_pcs(pcs: &[u8], state: u8, objs: &[&Object]) -> Vec<u8> {
        let mut p = pcs[..11].to_vec();
        p[7] = state;
        p[8] = 0; // palette update only
        p[10] = objs.len() as u8;
        for o in objs {
            p.extend_from_slice(&o.id.to_be_bytes());
            p.push(o.window);
            p.push(o.flags);
            p.extend_from_slice(&o.rest);
        }
        p
    }

    #[derive(Debug, Default, PartialEq, Eq)]
    pub struct Report {
        pub captions: usize,
        pub forced: usize,
    }

    /// Keep only the forced captions of a PGS stream. Each kept caption is
    /// written as a self-contained epoch (its window, palette and bitmaps
    /// included, even when the original reused ones defined earlier), and
    /// the display set that clears it follows it. A stream with no forced
    /// captions gives a single empty display set, so the output is still a
    /// valid (silent) track.
    pub fn forced_only(data: &[u8]) -> Result<(Vec<u8>, Report), String> {
        let segs = segments(data)?;
        // Latest definitions seen, by id.
        let mut windows: Option<Segment> = None;
        let mut palettes: HashMap<u8, Segment> = HashMap::new();
        let mut bitmaps: HashMap<u16, Vec<Segment>> = HashMap::new();
        let mut out = Vec::new();
        let mut report = Report::default();
        let mut showing = false;
        let mut first_pcs: Option<Segment> = None;

        // Group segments into display sets: PCS ... END.
        let mut i = 0;
        while i < segs.len() {
            if segs[i].kind != PCS {
                i += 1;
                continue;
            }
            let start = i;
            let mut end = i + 1;
            while end < segs.len() && segs[end].kind != END {
                end += 1;
            }
            let set = &segs[start..end.min(segs.len())];
            let end_seg = segs.get(end).cloned();
            i = end + 1;

            let pcs = &set[0];
            first_pcs.get_or_insert_with(|| pcs.clone());
            // Record definitions from this set (ODS may span several segments).
            let mut ods_in_set: HashMap<u16, Vec<Segment>> = HashMap::new();
            for s in &set[1..] {
                match s.kind {
                    WDS => windows = Some(s.clone()),
                    PDS if !s.payload.is_empty() => {
                        palettes.insert(s.payload[0], s.clone());
                    }
                    ODS if s.payload.len() >= 4 => {
                        let id = u16::from_be_bytes([s.payload[0], s.payload[1]]);
                        let first = s.payload[3] & 0x80 != 0;
                        let e = ods_in_set.entry(id).or_default();
                        if first {
                            e.clear();
                        }
                        e.push(s.clone());
                    }
                    _ => {}
                }
            }
            for (id, parts) in ods_in_set {
                bitmaps.insert(id, parts);
            }

            let objs = objects(&pcs.payload);
            if objs.is_empty() {
                // A clear: keep it only to take a kept caption off screen.
                if showing {
                    out.extend(set.iter().cloned());
                    out.extend(end_seg);
                    showing = false;
                }
                continue;
            }
            report.captions += 1;
            let forced: Vec<&Object> = objs.iter().filter(|o| o.flags & FORCED != 0).collect();
            if forced.is_empty() {
                if showing {
                    // A non-forced caption replaces a forced one: clear it.
                    let mut clear = pcs.clone();
                    clear.payload = rebuild_pcs(&pcs.payload, 0, &[]);
                    out.push(clear);
                    out.extend(windows.clone());
                    out.extend(end_seg);
                    showing = false;
                }
                continue;
            }
            report.forced += 1;
            // Self-contained epoch: PCS, window, palette, the bitmaps used.
            let mut head = pcs.clone();
            head.payload = rebuild_pcs(&pcs.payload, EPOCH_START, &forced);
            out.push(head);
            out.extend(windows.clone().map(|mut w| {
                w.pts = pcs.pts;
                w.dts = pcs.dts;
                w
            }));
            let palette_id = pcs.payload.get(9).copied().unwrap_or(0);
            if let Some(p) = palettes.get(&palette_id) {
                let mut p = p.clone();
                p.pts = pcs.pts;
                p.dts = pcs.dts;
                out.push(p);
            }
            for o in &forced {
                for part in bitmaps.get(&o.id).into_iter().flatten() {
                    let mut part = part.clone();
                    part.pts = pcs.pts;
                    part.dts = pcs.dts;
                    out.push(part);
                }
            }
            out.extend(end_seg);
            showing = true;
        }

        if report.forced == 0 {
            out.clear();
            if let Some(mut pcs) = first_pcs {
                pcs.payload = rebuild_pcs(&pcs.payload, EPOCH_START, &[]);
                out.push(pcs.clone());
                out.push(Segment {
                    kind: END,
                    payload: vec![],
                    ..pcs
                });
            }
        }
        let mut bytes = Vec::new();
        for s in &out {
            s.write(&mut bytes);
        }
        Ok((bytes, report))
    }

    /// (time, composition number, state, objects (id, forced), define bitmaps)
    #[cfg(test)]
    pub(crate) type TestSet<'a> = (f64, u16, u8, &'a [(u16, bool)], bool);

    #[cfg(test)]
    pub(crate) fn build(sets: &[TestSet]) -> Vec<u8> {
        // (time, composition number, state, objects (id, forced), define bitmaps)
        let mut out = Vec::new();
        for (t, num, state, objs, define) in sets {
            let pts = (t * 90000.0) as u32;
            let mut pcs = vec![0x07, 0x80, 0x04, 0x38, 0x10];
            pcs.extend_from_slice(&num.to_be_bytes());
            pcs.extend_from_slice(&[*state, 0, 0, objs.len() as u8]);
            for (id, forced) in objs.iter() {
                pcs.extend_from_slice(&id.to_be_bytes());
                pcs.extend_from_slice(&[0, if *forced { FORCED } else { 0 }, 0, 100, 3, 0x84]);
            }
            let mut set = vec![Segment {
                pts,
                dts: pts,
                kind: PCS,
                payload: pcs,
            }];
            if *define {
                set.push(Segment {
                    pts,
                    dts: pts,
                    kind: WDS,
                    payload: vec![1, 0, 0, 100, 3, 0x84, 0, 200, 0, 40],
                });
                set.push(Segment {
                    pts,
                    dts: pts,
                    kind: PDS,
                    payload: vec![0, 0, 1, 200, 128, 128, 255],
                });
                for (id, _) in objs.iter() {
                    let mut ods = id.to_be_bytes().to_vec();
                    ods.extend_from_slice(&[0, 0xC0, 0, 0, 6, 0, 1, 0, 1, 1, 0]);
                    set.push(Segment {
                        pts,
                        dts: pts,
                        kind: ODS,
                        payload: ods,
                    });
                }
            }
            set.push(Segment {
                pts,
                dts: pts,
                kind: END,
                payload: vec![],
            });
            for s in set {
                s.write(&mut out);
            }
        }
        out
    }

    #[cfg(test)]
    pub(crate) fn kinds(data: &[u8]) -> Vec<(u32, u8)> {
        segments(data)
            .unwrap()
            .into_iter()
            .map(|s| (s.pts / 90000, s.kind))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(index: u32, lang: &str, forced: bool, default: bool) -> SubtitleTrack {
        SubtitleTrack {
            index,
            pid: None,
            codec: SubtitleCodec::Pgs,
            language: Some(lang.into()),
            title: None,
            forced,
            default,
            kbps: None,
        }
    }

    #[test]
    fn default_plan_and_edits() {
        let tracks = vec![
            sub(3, "eng", false, true),
            sub(4, "fre", false, true),
            sub(5, "ger", true, false),
        ];
        let rules = SubtitleRules {
            languages: vec!["eng".into()],
            keep_forced: true,
        };
        let mut plan = default_subtitle_plan(&tracks, &rules);
        assert_eq!(plan.len(), 2, "English plus the forced German track");
        assert!(plan[0].default && !plan[1].default);
        assert_eq!(
            plan[1].source,
            SubSource::Track {
                index: 5,
                ordinal: 2
            }
        );

        // Default moves to the edited track; forced-only implies forced.
        edit_subtitle(
            &mut plan,
            1,
            &SubEdit {
                default: true,
                forced_only: true,
                language: Some("DE".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!plan[0].default && plan[1].default && plan[1].forced);
        assert_eq!(plan[1].language.as_deref(), Some("de"));
        assert!(plan[1].describe().ends_with("forced captions only"));
        assert!(normalise_language("english").is_err());

        add_source_subtitle(&mut plan, &tracks, 1).unwrap();
        assert!(!plan[2].default);
        move_subtitle(&mut plan, 2, 0).unwrap();
        assert_eq!(plan[0].language.as_deref(), Some("fre"));
        remove_subtitle(&mut plan, 0).unwrap();
        assert_eq!(plan.len(), 2);
    }

    #[test]
    fn subtitle_files() {
        let dir = std::env::temp_dir().join(format!("anvil-subs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let srt = dir.join("Film (2026).en.forced.srt");
        std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:02,000\nHi\n").unwrap();
        let mut plan = Vec::new();
        add_file(&mut plan, &srt).unwrap();
        assert_eq!(plan[0].codec, SubtitleCodec::Srt);
        assert_eq!(plan[0].language.as_deref(), Some("en"));
        assert!(plan[0].forced);
        // Forced-only needs a PGS track from the source.
        assert!(edit_subtitle(
            &mut plan,
            0,
            &SubEdit {
                forced_only: true,
                ..Default::default()
            }
        )
        .is_err());
        assert!(add_file(&mut plan, &dir.join("missing.srt")).is_err());
        assert!(add_file(&mut plan, &dir.join("notes.txt")).is_err());
        let idx = dir.join("Film.idx");
        std::fs::write(&idx, "# VobSub index").unwrap();
        assert!(add_file(&mut plan, &idx).is_err(), "no .sub beside it");

        let tracks = vec![sub(3, "eng", false, false)];
        let mut plan = Vec::new();
        parse_sub_spec("1,forced-only,title=Forced", &tracks, &mut plan).unwrap();
        parse_sub_spec(
            &format!("{},lang=eng,default", srt.display()),
            &tracks,
            &mut plan,
        )
        .unwrap();
        assert!(plan[0].forced_only && plan[0].title.as_deref() == Some("Forced"));
        assert!(plan[1].default && plan[1].language.as_deref() == Some("eng"));
        assert!(parse_sub_spec("1,shouty", &tracks, &mut plan).is_err());
    }

    #[test]
    fn pgs_forced_only() {
        let stream = pgs::build(&[
            (1.0, 0, 0x80, &[(0, false)], true),
            (2.0, 1, 0x00, &[], false),
            (3.0, 2, 0x80, &[(1, true)], true),
            (4.0, 3, 0x00, &[], false),
            // Forced, reusing object 0 defined at 1 s without redefining it.
            (7.0, 6, 0x00, &[(0, true)], false),
            (8.0, 7, 0x00, &[], false),
            // Mixed: only the forced object stays.
            (9.0, 8, 0x80, &[(2, false), (3, true)], true),
            (10.0, 9, 0x00, &[], false),
        ]);
        let (out, report) = pgs::forced_only(&stream).unwrap();
        assert_eq!(
            report,
            pgs::Report {
                captions: 4,
                forced: 3
            }
        );
        let k = pgs::kinds(&out);
        // 1 s (normal) is gone; 3 s shows and 4 s clears it.
        assert_eq!(k[0], (3, 0x16));
        assert!(k.contains(&(4, 0x16)));
        assert!(!k.iter().any(|(t, _)| *t == 1 || *t == 2));
        // 7 s carries object 0's bitmap and the palette copied from 1 s.
        let at7: Vec<u8> = k
            .iter()
            .filter(|(t, _)| *t == 7)
            .map(|(_, kind)| *kind)
            .collect();
        assert_eq!(at7, vec![0x16, 0x17, 0x14, 0x15, 0x80]);
        // 9 s keeps one object (bitmap 3 only).
        let at9: Vec<u8> = k
            .iter()
            .filter(|(t, _)| *t == 9)
            .map(|(_, kind)| *kind)
            .collect();
        assert_eq!(at9, vec![0x16, 0x17, 0x14, 0x15, 0x80]);

        // Nothing forced: one silent display set, still a valid stream.
        let none = pgs::build(&[
            (1.0, 0, 0x80, &[(0, false)], true),
            (2.0, 1, 0x00, &[], false),
        ]);
        let (out, report) = pgs::forced_only(&none).unwrap();
        assert_eq!(report.forced, 0);
        assert_eq!(pgs::kinds(&out), vec![(1, 0x16), (1, 0x80)]);
        assert!(pgs::forced_only(b"not pgs at all").is_err());
    }
}
