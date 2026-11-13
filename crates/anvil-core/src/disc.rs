// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Blu-ray and DVD folders: find the main title and copy it to one MKV.
//!
//! A disc folder is opened by copying its main title (no re-encoding) into a
//! single Matroska file with mkvmerge, which then goes through the normal
//! probe, crop and plan path. mkvmerge does the disc-specific work: it reads
//! Blu-ray MPLS playlists (joining seamless-branching clips and taking the
//! chapters from the playlist), and follows a DVD's VOB set on its own.
//!
//! The main title is the longest playlist (Blu-ray) or the largest title set
//! (DVD). Choosing another title is a later phase. Encrypted discs are out of
//! scope: rip them with MakeMKV first.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::command::Command;
use crate::dvd::{DvdTitle, SECTOR};
use crate::error::{Error, Result};
use crate::process::run_with_timeout;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscKind {
    BluRay,
    Dvd,
}

impl DiscKind {
    pub fn name(self) -> &'static str {
        match self {
            DiscKind::BluRay => "Blu-ray",
            DiscKind::Dvd => "DVD",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscTitle {
    /// Playlist number ("00800") or title set ("01").
    pub id: String,
    /// What mkvmerge opens: the playlist, or the title set's first VOB.
    pub source: PathBuf,
    pub files: Vec<PathBuf>,
    pub duration_secs: Option<f64>,
    pub size_bytes: u64,
    pub chapters: u32,
    /// DVD titles read from the IFO: cells, chapters and languages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dvd: Option<DvdTitle>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Disc {
    pub kind: DiscKind,
    /// The folder holding BDMV or VIDEO_TS.
    pub root: PathBuf,
    pub label: String,
    pub titles: Vec<DiscTitle>,
}

/// Is this a disc folder (or a file inside one's top level)? Accepts the
/// disc's folder, its BDMV or VIDEO_TS folder, or index.bdmv / VIDEO_TS.IFO.
pub fn detect(path: &Path) -> Option<(DiscKind, PathBuf)> {
    let name_is = |p: &Path, n: &str| {
        p.file_name()
            .is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(n))
    };
    let mut p = path.to_path_buf();
    if p.is_file() && (name_is(&p, "index.bdmv") || name_is(&p, "VIDEO_TS.IFO")) {
        p = p.parent()?.to_path_buf();
    }
    if p.is_dir() && (name_is(&p, "BDMV") || name_is(&p, "VIDEO_TS")) {
        p = p.parent()?.to_path_buf();
    }
    if child(&p, "BDMV").is_some_and(|b| child(&b, "PLAYLIST").is_some()) {
        return Some((DiscKind::BluRay, p));
    }
    if child(&p, "VIDEO_TS").is_some() {
        return Some((DiscKind::Dvd, p));
    }
    None
}

/// A child folder or file by name, ignoring case (discs copied from
/// Windows sometimes have lower-case names).
fn child(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|c| {
            c.file_name()
                .is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(name))
        })
}

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// List the disc's titles. Blu-ray playlists are read with `mkvmerge -J`.
pub fn scan(path: &Path, timeout: Duration) -> Result<Disc> {
    let (kind, root) = detect(path).ok_or_else(|| Error::Invalid {
        field: "disc",
        reason: format!("{} is not a Blu-ray or DVD folder", path.display()),
    })?;
    let label = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Disc".into());
    let titles = match kind {
        DiscKind::BluRay => scan_bluray(&root, timeout)?,
        DiscKind::Dvd => scan_dvd(&root),
    };
    if titles.is_empty() {
        return Err(Error::Invalid {
            field: "disc",
            reason: format!(
                "no readable titles in {} (is it encrypted? Rip it with MakeMKV first)",
                root.display()
            ),
        });
    }
    Ok(Disc {
        kind,
        root,
        label,
        titles,
    })
}

fn scan_bluray(root: &Path, timeout: Duration) -> Result<Vec<DiscTitle>> {
    let playlists = child(root, "BDMV")
        .and_then(|b| child(&b, "PLAYLIST"))
        .map(|p| sorted_entries(&p))
        .unwrap_or_default();
    let mut titles: Vec<DiscTitle> = Vec::new();
    for mpls in playlists.into_iter().filter(|p| {
        p.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mpls"))
    }) {
        let out = run_with_timeout(
            "mkvmerge",
            &["-J".to_string(), mpls.to_string_lossy().into_owned()],
            timeout,
        )?;
        if let Some(t) = parse_playlist(&mpls, &out.stdout) {
            // Discs often carry the same title under several playlists.
            if !titles
                .iter()
                .any(|o| o.files == t.files && o.duration_secs == t.duration_secs)
            {
                titles.push(t);
            }
        }
    }
    Ok(titles)
}

/// One playlist from `mkvmerge -J playlist.mpls`.
pub fn parse_playlist(mpls: &Path, json: &str) -> Option<DiscTitle> {
    let v: Value = serde_json::from_str(json).ok()?;
    let props = v.get("container")?.get("properties")?;
    if props.get("playlist").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let files: Vec<PathBuf> = props
        .get("playlist_file")?
        .as_array()?
        .iter()
        .filter_map(|f| f.as_str().map(PathBuf::from))
        .collect();
    if files.is_empty() {
        return None;
    }
    Some(DiscTitle {
        id: mpls.file_stem()?.to_string_lossy().into_owned(),
        source: mpls.to_path_buf(),
        files,
        duration_secs: props
            .get("playlist_duration")
            .and_then(Value::as_u64)
            .map(|ns| ns as f64 / 1e9),
        size_bytes: props
            .get("playlist_size")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        chapters: props
            .get("playlist_chapters")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        dvd: None,
    })
}

/// DVD title sets: VTS_nn_1.VOB onwards (VTS_nn_0.VOB is the menu).
fn scan_dvd(root: &Path) -> Vec<DiscTitle> {
    let Some(vts) = child(root, "VIDEO_TS") else {
        return Vec::new();
    };
    let mut sets: std::collections::BTreeMap<String, Vec<(u32, PathBuf, u64)>> = Default::default();
    for p in sorted_entries(&vts) {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_uppercase())
            .unwrap_or_default();
        // VTS_01_1.VOB
        let Some(rest) = name
            .strip_prefix("VTS_")
            .and_then(|r| r.strip_suffix(".VOB"))
        else {
            continue;
        };
        let Some((set, part)) = rest.split_once('_') else {
            continue;
        };
        let Ok(part) = part.parse::<u32>() else {
            continue;
        };
        if part == 0 {
            continue;
        }
        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        sets.entry(set.to_string())
            .or_default()
            .push((part, p, size));
    }
    let mut titles = Vec::new();
    for (set, mut parts) in sets {
        parts.sort_by_key(|(n, _, _)| *n);
        let files: Vec<PathBuf> = parts.iter().map(|(_, p, _)| p.clone()).collect();
        let ifo = child(&vts, &format!("VTS_{set}_0.IFO"))
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|d| crate::dvd::parse_vts_ifo(&d).ok())
            .filter(|t| !t.is_empty());
        match ifo {
            Some(dvd_titles) => {
                let single = dvd_titles.len() == 1;
                for t in dvd_titles {
                    titles.push(DiscTitle {
                        id: if single {
                            set.clone()
                        } else {
                            format!("{set}.{}", t.number)
                        },
                        source: files[0].clone(),
                        files: files.clone(),
                        duration_secs: Some(t.duration_secs),
                        size_bytes: t.sectors() * SECTOR,
                        chapters: t.chapters.len() as u32,
                        dvd: Some(t),
                    });
                }
            }
            // No readable IFO: the whole title set, as mkvmerge reads it.
            None => titles.push(DiscTitle {
                id: set,
                // mkvmerge follows VTS_nn_2.VOB onwards by itself; naming
                // them too would join the film twice.
                source: files[0].clone(),
                size_bytes: parts.iter().map(|(_, _, s)| s).sum(),
                files,
                duration_secs: None,
                chapters: 0,
                dvd: None,
            }),
        }
    }
    titles
}

/// The main feature: the longest playlist, or the largest DVD title set.
pub fn main_title(disc: &Disc) -> Option<&DiscTitle> {
    disc.titles.iter().max_by(|a, b| {
        // Whole seconds, so near-identical decoy playlists fall back to size.
        let secs = |t: &DiscTitle| t.duration_secs.map_or(0, |d| d as u64);
        secs(a)
            .cmp(&secs(b))
            .then(a.size_bytes.cmp(&b.size_bytes))
            // Prefer the lower playlist number on a full tie.
            .then(b.id.cmp(&a.id))
    })
}

impl DiscTitle {
    pub fn describe(&self, kind: DiscKind) -> String {
        let what = match kind {
            DiscKind::BluRay => format!("playlist {}", self.id),
            DiscKind::Dvd => format!("title {}", self.id),
        };
        let mut parts = vec![what];
        if let Some(d) = self.duration_secs {
            parts.push(crate::progress::format_duration(d as u64));
        }
        parts.push(crate::sizing::gb(self.size_bytes));
        match self.chapters {
            0 => {}
            1 => parts.push("1 chapter".into()),
            n => parts.push(format!("{n} chapters")),
        }
        parts.join(", ")
    }

    /// Things to tell the user about what the copy will and will not have.
    pub fn notes(&self, kind: DiscKind) -> Vec<String> {
        let mut notes = Vec::new();
        if self.dvd.is_none() && kind == DiscKind::Dvd {
            notes.push(
                "The disc's IFO could not be read, so the whole title set is copied and track languages are not known."
                    .into(),
            );
        }
        notes
    }
}

/// Where the copied title goes: beside the disc folder, named after it.
pub fn remux_path(disc: &Disc, title: &DiscTitle, dir: &Path) -> PathBuf {
    let what = match disc.kind {
        DiscKind::BluRay => title.id.clone(),
        DiscKind::Dvd => format!("title {}", title.id),
    };
    dir.join(format!("{} - {what}.mkv", disc.label))
}

/// Copy a title into one MKV without re-encoding. `--gui-mode` makes
/// mkvmerge print `#GUI#progress N%` lines.
pub fn remux_command(title: &DiscTitle, output: &Path) -> Command {
    Command::new("mkvmerge").args([
        "--gui-mode".to_string(),
        "-o".to_string(),
        output.to_string_lossy().into_owned(),
        title.source.to_string_lossy().into_owned(),
    ])
}

/// Copy a title to `output` with progress (0 to 1). It is written under a
/// temporary name and renamed when complete, so an existing `output` is
/// always a finished copy and is reused rather than copied again. Setting
/// `cancel` stops the copy and removes the partial file.
pub fn copy_title(
    title: &DiscTitle,
    output: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(f64),
) -> Result<()> {
    if output.is_file() && std::fs::metadata(output).is_ok_and(|m| m.len() > 0) {
        on_progress(1.0);
        return Ok(());
    }
    match &title.dvd {
        Some(d) => copy_dvd_title(title, d, output, cancel, on_progress),
        None => {
            let partial = output.with_extension("partial.mkv");
            run_mkvmerge(
                &remux_command(title, &partial),
                &partial,
                output,
                cancel,
                on_progress,
            )
        }
    }
}

/// A DVD title: copy exactly its cells (in play order, first angle) into
/// one VOB, then mux that with the IFO's languages and chapter points.
fn copy_dvd_title(
    title: &DiscTitle,
    d: &DvdTitle,
    output: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(f64),
) -> Result<()> {
    let vob = output.with_extension("partial.vob");
    let xml = output.with_extension("chapters.xml");
    let mut extra: Vec<PathBuf> = Vec::new();
    let result = (|| {
        copy_cells(&title.files, &d.cells, &vob, cancel, |p| {
            on_progress(p * 0.5)
        })?;
        let mut cmd = Command::new("mkvmerge").args(["--gui-mode", "-o"]);
        let partial = output.with_extension("partial.mkv");
        cmd = cmd.arg(partial.to_string_lossy().into_owned());
        if !d.chapters.is_empty() {
            let list: Vec<crate::chapters::Chapter> = d
                .chapters
                .iter()
                .enumerate()
                .map(|(i, t)| crate::chapters::Chapter {
                    start_secs: *t,
                    title: crate::chapters::default_title(i),
                })
                .collect();
            let lang = d
                .audio
                .first()
                .and_then(|a| a.language.clone())
                .unwrap_or_else(|| "eng".into());
            std::fs::write(&xml, crate::chapters::to_matroska_xml(&list, &lang)).map_err(|e| {
                Error::Io {
                    path: xml.clone(),
                    source: e,
                }
            })?;
            cmd = cmd.args(["--chapters".to_string(), xml.to_string_lossy().into_owned()]);
        }
        let ident = run_with_timeout(
            "mkvmerge",
            &["-J".to_string(), vob.to_string_lossy().into_owned()],
            Duration::from_secs(120),
        )?;
        for (tid, lang) in dvd_languages(&ident.stdout, d) {
            cmd = cmd.args(["--language".to_string(), format!("{tid}:{lang}")]);
        }
        cmd = cmd.arg(vob.to_string_lossy().into_owned());
        // Subtitles: mkvmerge cannot read them from the VOB, so they are
        // pulled out here as VobSub pairs carrying the IFO's palette.
        if !d.subtitles.is_empty() {
            let file = std::fs::File::open(&vob).map_err(|e| Error::Io {
                path: vob.clone(),
                source: e,
            })?;
            let found =
                crate::vobsub::extract(&mut std::io::BufReader::new(file)).map_err(|e| {
                    Error::Io {
                        path: vob.clone(),
                        source: e,
                    }
                })?;
            let base = found.first_video_pts.unwrap_or(0);
            for (n, spus) in &found.streams {
                let lang = d
                    .subtitles
                    .iter()
                    .find(|s| s.stream_id == 0x20 + u32::from(*n))
                    .and_then(|s| s.language.clone());
                let idx = output.with_extension(format!("subs{n}.idx"));
                crate::vobsub::write_idx_sub(
                    spus,
                    base,
                    &d.palette,
                    (720, d.height),
                    lang.as_deref(),
                    &idx,
                )
                .map_err(|e| Error::Io {
                    path: idx.clone(),
                    source: e,
                })?;
                if let Some(l) = &lang {
                    cmd = cmd.args(["--language".to_string(), format!("0:{l}")]);
                }
                cmd = cmd.arg(idx.to_string_lossy().into_owned());
                extra.push(idx);
            }
        }
        run_mkvmerge(&cmd, &partial, output, cancel, |p| {
            on_progress(0.5 + p * 0.5)
        })
    })();
    let _ = std::fs::remove_file(&vob);
    let _ = std::fs::remove_file(&xml);
    for idx in extra {
        let _ = std::fs::remove_file(idx.with_extension("sub"));
        let _ = std::fs::remove_file(idx);
    }
    result
}

/// mkvmerge track ID to language, matching each audio track's MPEG-PS
/// stream identity with the IFO's streams.
pub fn dvd_languages(mkvmerge_json: &str, d: &DvdTitle) -> Vec<(u64, String)> {
    let Ok(v) = serde_json::from_str::<Value>(mkvmerge_json) else {
        return Vec::new();
    };
    v.get("tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let id = t.get("id")?.as_u64()?;
            let p = t.get("properties")?;
            let stream = p
                .get("sub_stream_id")
                .and_then(Value::as_u64)
                .or_else(|| p.get("stream_id").and_then(Value::as_u64))?;
            let lang = d
                .audio
                .iter()
                .find(|a| u64::from(a.stream_id) == stream)?
                .language
                .clone()?;
            Some((id, lang))
        })
        .collect()
}

/// Copy sector ranges out of a VOB set (one long run of sectors split over
/// files of any size) into `out`.
fn copy_cells(
    files: &[PathBuf],
    cells: &[(u32, u32)],
    out: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(f64),
) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom, Write};
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |e: std::io::Error| Error::Io {
            path: path.clone(),
            source: e,
        }
    };
    // Each file's start offset in the set.
    let mut spans = Vec::new();
    let mut start = 0u64;
    for f in files {
        let len = std::fs::metadata(f).map_err(io(f))?.len();
        spans.push((start, len, f));
        start += len;
    }
    let total: u64 = cells
        .iter()
        .map(|(a, b)| u64::from(b - a + 1) * SECTOR)
        .sum();
    let mut writer = std::io::BufWriter::new(std::fs::File::create(out).map_err(io(out))?);
    let mut done = 0u64;
    let mut buf = vec![0u8; 4 << 20];
    for (first, last) in cells {
        let mut pos = u64::from(*first) * SECTOR;
        let end = (u64::from(*last) + 1) * SECTOR;
        while pos < end {
            if cancel.load(Ordering::SeqCst) {
                drop(writer);
                let _ = std::fs::remove_file(out);
                return Err(Error::Cancelled);
            }
            let Some((fstart, flen, f)) = spans.iter().find(|(s, l, _)| pos >= *s && pos < s + l)
            else {
                return Err(Error::Invalid {
                    field: "disc",
                    reason: format!("sector {} is past the end of the VOB files", pos / SECTOR),
                });
            };
            let n = (end - pos).min(fstart + flen - pos).min(buf.len() as u64) as usize;
            let mut file = std::fs::File::open(f).map_err(io(f))?;
            file.seek(SeekFrom::Start(pos - fstart)).map_err(io(f))?;
            file.read_exact(&mut buf[..n]).map_err(io(f))?;
            writer.write_all(&buf[..n]).map_err(io(out))?;
            pos += n as u64;
            done += n as u64;
            on_progress(done as f64 / total.max(1) as f64);
        }
    }
    writer.flush().map_err(io(out))?;
    Ok(())
}

/// Run a mkvmerge `--gui-mode` command writing `partial`, then rename it
/// to `output`.
fn run_mkvmerge(
    cmd: &Command,
    partial: &Path,
    output: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(f64),
) -> Result<()> {
    let io = |e: std::io::Error| Error::Io {
        path: partial.to_path_buf(),
        source: e,
    };
    let mut child = std::process::Command::new(&cmd.program)
        .args(&cmd.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| Error::ToolMissing(format!("mkvmerge ({e})")))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let mut errors = Vec::new();
    // Blu-ray playlists are scanned first, with their own 0 to 100%.
    let mut scanning = false;
    // Progress lines arrive often; cancel is checked on each one.
    for line in BufReader::new(stdout)
        .lines()
        .map_while(std::result::Result::ok)
    {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            break;
        }
        if line.starts_with("#GUI#begin_scanning_playlists") {
            scanning = true;
        } else if line.starts_with("#GUI#end_scanning_playlists") {
            scanning = false;
        } else if let (false, Some(p)) = (scanning, parse_remux_progress(&line)) {
            on_progress(p);
        } else if let Some(e) = line.strip_prefix("#GUI#error ") {
            errors.push(e.to_string());
        }
    }
    let status = child.wait().map_err(io)?;
    if cancel.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(partial);
        return Err(Error::Cancelled);
    }
    // mkvmerge exits 1 for warnings only; 2 is an error.
    if status.code().is_some_and(|c| c <= 1) && partial.is_file() {
        std::fs::rename(partial, output).map_err(io)?;
        on_progress(1.0);
        Ok(())
    } else {
        let _ = std::fs::remove_file(partial);
        Err(Error::Tool {
            tool: "mkvmerge".into(),
            message: if errors.is_empty() {
                format!("copying the title failed ({status})")
            } else {
                errors.join("; ")
            },
        })
    }
}

/// Percentage from a `mkvmerge --gui-mode` line.
pub fn parse_remux_progress(line: &str) -> Option<f64> {
    let pct = line
        .trim()
        .strip_prefix("#GUI#progress ")?
        .strip_suffix('%')?;
    pct.trim().parse::<f64>().ok().map(|p| p / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("anvil-disc-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn detects_disc_folders_from_any_level() {
        let root = tmp("bd");
        let pl = root.join("BDMV").join("PLAYLIST");
        std::fs::create_dir_all(&pl).unwrap();
        std::fs::write(root.join("BDMV").join("index.bdmv"), b"INDX").unwrap();
        for p in [
            root.clone(),
            root.join("BDMV"),
            root.join("BDMV").join("index.bdmv"),
        ] {
            assert_eq!(
                detect(&p),
                Some((DiscKind::BluRay, root.clone())),
                "{}",
                p.display()
            );
        }
        let dvd = tmp("dvd");
        std::fs::create_dir_all(dvd.join("video_ts")).unwrap();
        assert_eq!(detect(&dvd), Some((DiscKind::Dvd, dvd.clone())));
        assert_eq!(
            detect(&dvd.join("video_ts")),
            Some((DiscKind::Dvd, dvd.clone()))
        );
        assert_eq!(detect(&std::env::temp_dir().join("nope-not-here")), None);
    }

    #[test]
    fn dvd_title_sets_skip_menus_and_pick_the_largest() {
        let root = tmp("dvdscan");
        let v = root.join("VIDEO_TS");
        std::fs::create_dir_all(&v).unwrap();
        for (name, size) in [
            ("VIDEO_TS.VOB", 50),
            ("VTS_01_0.VOB", 900),
            ("VTS_01_1.VOB", 1000),
            ("VTS_01_2.VOB", 1000),
            ("VTS_02_1.VOB", 1500),
        ] {
            std::fs::write(v.join(name), vec![0u8; size]).unwrap();
        }
        let disc = scan(&root, Duration::from_secs(5)).unwrap();
        assert_eq!(disc.kind, DiscKind::Dvd);
        assert_eq!(disc.titles.len(), 2);
        let main = main_title(&disc).unwrap();
        assert_eq!(main.id, "01");
        assert_eq!(main.size_bytes, 2000);
        assert_eq!(main.files.len(), 2);
        assert!(main.source.ends_with("VTS_01_1.VOB"));
        // Only the first VOB is given to mkvmerge.
        let cmd = remux_command(main, Path::new("/out/x.mkv"));
        assert_eq!(cmd.args.len(), 4);
        assert_eq!(
            remux_path(&disc, main, Path::new("/out")),
            PathBuf::from(format!("/out/{} - title 01.mkv", disc.label))
        );
    }

    #[test]
    fn playlist_json_and_main_title_choice() {
        let json = |ms: u64, size: u64, files: &[&str]| {
            serde_json::json!({"container": {"properties": {
                "playlist": true, "playlist_duration": ms * 1_000_000,
                "playlist_size": size, "playlist_file": files, "playlist_chapters": 24
            }}})
            .to_string()
        };
        let a = parse_playlist(
            Path::new("/d/BDMV/PLAYLIST/00800.mpls"),
            &json(
                7_920_000,
                48_000_000_000,
                &["/d/BDMV/STREAM/00001.m2ts", "/d/BDMV/STREAM/00002.m2ts"],
            ),
        )
        .unwrap();
        assert_eq!(a.id, "00800");
        assert_eq!(a.duration_secs, Some(7920.0));
        assert_eq!(a.chapters, 24);
        assert_eq!(
            a.describe(DiscKind::BluRay),
            "playlist 00800, 2:12:00, 48.0 GB, 24 chapters"
        );
        let extra = parse_playlist(
            Path::new("/d/BDMV/PLAYLIST/00001.mpls"),
            &json(300_000, 900_000_000, &["/d/BDMV/STREAM/00010.m2ts"]),
        )
        .unwrap();
        // A decoy with the same length but a few frames short.
        let decoy = DiscTitle {
            id: "00801".into(),
            duration_secs: Some(7919.6),
            size_bytes: 47_000_000_000,
            ..a.clone()
        };
        let disc = Disc {
            kind: DiscKind::BluRay,
            root: "/d".into(),
            label: "FILM".into(),
            titles: vec![extra, decoy, a.clone()],
        };
        assert_eq!(main_title(&disc).unwrap().id, "00800");
        assert!(
            parse_playlist(Path::new("x.mpls"), r#"{"container":{"properties":{}}}"#).is_none()
        );
        assert_eq!(parse_remux_progress("#GUI#progress 34%"), Some(0.34));
        assert_eq!(parse_remux_progress("Progress: 34%"), None);
    }
}
