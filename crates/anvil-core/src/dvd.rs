// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! DVD title sets: reading `VTS_nn_0.IFO`.
//!
//! A DVD's video lives in the title set's VOB files; the IFO says how to
//! play it. Each title is a program chain (PGC) made of cells: sector
//! ranges of the VOBs, in order. Programs inside the chain are the
//! chapters. The IFO also names each audio and subpicture stream's language.
//! This reads only what Anvil needs: titles, their cells (first angle
//! only), lengths, chapter points, and stream languages.

use serde::{Deserialize, Serialize};

pub const SECTOR: u64 = 2048;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DvdStream {
    /// MPEG-PS stream identity as mkvmerge reports it: AC-3 0x80+n,
    /// DTS 0x88+n and LPCM 0xA0+n are private-stream sub-ids; MPEG audio is
    /// stream id 0xC0+n; subpictures are sub-ids 0x20+n.
    pub stream_id: u32,
    pub language: Option<String>,
    pub format: String,
    pub channels: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DvdTitle {
    /// Title number within the title set (1 = first).
    pub number: u32,
    pub duration_secs: f64,
    /// Sector ranges (inclusive) in the title set's VOBs, in play order.
    pub cells: Vec<(u32, u32)>,
    /// Chapter start times (seconds).
    pub chapters: Vec<f64>,
    pub audio: Vec<DvdStream>,
    pub subtitles: Vec<DvdStream>,
    /// The subpicture colours: 16 entries of 0x00YYCrCb.
    #[serde(default)]
    pub palette: Vec<u32>,
    /// Picture height: 480 (NTSC) or 576 (PAL).
    #[serde(default = "default_height")]
    pub height: u32,
}

fn default_height() -> u32 {
    480
}

impl DvdTitle {
    pub fn sectors(&self) -> u64 {
        self.cells.iter().map(|(a, b)| u64::from(b - a + 1)).sum()
    }
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

fn from_bcd(b: u8) -> f64 {
    f64::from((b >> 4) * 10 + (b & 0x0F))
}

/// DVD playback time: BCD hours, minutes, seconds, then frames with the
/// frame rate in the top two bits (01 = 25 fps, 11 = 30 fps).
pub fn playback_time(b: &[u8]) -> f64 {
    if b.len() < 4 {
        return 0.0;
    }
    let fps = if b[3] >> 6 == 0b01 {
        25.0
    } else {
        30000.0 / 1001.0
    };
    from_bcd(b[0]) * 3600.0 + from_bcd(b[1]) * 60.0 + from_bcd(b[2]) + from_bcd(b[3] & 0x3F) / fps
}

fn language(code: &[u8], present: bool) -> Option<String> {
    let s: String = code.iter().map(|c| *c as char).collect();
    (present && s.chars().all(|c| c.is_ascii_alphabetic())).then(|| s.to_ascii_lowercase())
}

/// Parse a title set IFO (`VTS_nn_0.IFO`).
pub fn parse_vts_ifo(d: &[u8]) -> Result<Vec<DvdTitle>, String> {
    if d.get(0..12) != Some(b"DVDVIDEO-VTS".as_slice()) {
        return Err("not a DVD title set IFO".into());
    }
    let bad = || "the IFO is truncated or damaged".to_string();

    // Stream attributes (logical stream n -> format and language).
    let n_audio = usize::from(d.get(0x203).copied().ok_or_else(bad)?).min(8);
    let audio_attr: Vec<(u8, u32, Option<String>)> = (0..n_audio)
        .map(|i| {
            let a = &d[0x204 + 8 * i..0x204 + 8 * i + 8];
            let format = a[0] >> 5;
            let lang = language(&a[2..4], (a[0] >> 2) & 3 == 1);
            (format, u32::from(a[1] & 7) + 1, lang)
        })
        .collect();
    let n_subp = usize::from(d.get(0x255).copied().ok_or_else(bad)?).min(32);
    let subp_lang: Vec<Option<String>> = (0..n_subp)
        .map(|i| {
            let s = &d[0x256 + 6 * i..0x256 + 6 * i + 6];
            language(&s[2..4], s[0] & 3 == 1)
        })
        .collect();

    // Video attributes: bits 5-4 of the first byte, 0 NTSC, 1 PAL.
    let height = if (d.get(0x200).copied().unwrap_or(0) >> 4) & 3 == 1 {
        576
    } else {
        480
    };
    let pgcit = u32_at(d, 0xCC).ok_or_else(bad)? as usize * SECTOR as usize;
    let n_pgc = usize::from(u16_at(d, pgcit).ok_or_else(bad)?);
    let mut titles = Vec::new();
    for i in 0..n_pgc {
        let srp = pgcit + 8 + 8 * i;
        let entry = *d.get(srp).ok_or_else(bad)?;
        // Only entry PGCs start a title; the rest are chained parts.
        if entry & 0x80 == 0 {
            continue;
        }
        let pgc = pgcit + u32_at(d, srp + 4).ok_or_else(bad)? as usize;
        let p = d.get(pgc..).ok_or_else(bad)?;
        if p.len() < 0xEC {
            return Err(bad());
        }
        let n_programs = usize::from(p[2]);
        let n_cells = usize::from(p[3]);
        if n_cells == 0 {
            continue;
        }
        let duration = playback_time(&p[4..8]);

        // Audio: available logical streams and their physical numbers.
        let mut audio = Vec::new();
        for (n, (format, channels, lang)) in audio_attr.iter().enumerate() {
            let ctl = u16_at(p, 0x0C + 2 * n).unwrap_or(0);
            if ctl & 0x8000 == 0 {
                continue;
            }
            let phys = u32::from((ctl >> 8) & 0x07);
            let (stream_id, name) = match format {
                0 => (0x80 + phys, "AC-3"),
                2 | 3 => (0xC0 + phys, "MPEG audio"),
                4 => (0xA0 + phys, "LPCM"),
                6 => (0x88 + phys, "DTS"),
                _ => continue,
            };
            audio.push(DvdStream {
                stream_id,
                language: lang.clone(),
                format: name.into(),
                channels: *channels,
            });
        }
        let mut subtitles = Vec::new();
        for (n, lang) in subp_lang.iter().enumerate() {
            let ctl = u32_at(p, 0x1C + 4 * n).unwrap_or(0);
            if ctl & 0x8000_0000 == 0 {
                continue;
            }
            // The 4:3 (or only) stream number.
            let phys = (ctl >> 24) & 0x1F;
            subtitles.push(DvdStream {
                stream_id: 0x20 + phys,
                language: lang.clone(),
                format: "VobSub".into(),
                channels: 0,
            });
        }

        let prog_off = usize::from(u16_at(p, 0xE6).ok_or_else(bad)?);
        let cell_off = usize::from(u16_at(p, 0xE8).ok_or_else(bad)?);
        let program_cells: Vec<usize> = (0..n_programs)
            .filter_map(|k| p.get(prog_off + k).map(|c| usize::from(*c)))
            .collect();
        let mut cells = Vec::new();
        let mut chapters = Vec::new();
        let mut t = 0.0;
        for c in 0..n_cells {
            let cp = p
                .get(cell_off + 24 * c..cell_off + 24 * c + 24)
                .ok_or_else(bad)?;
            let block_mode = cp[0] >> 6;
            let block_type = (cp[0] >> 4) & 3;
            // Angle blocks: keep the first angle's cell only.
            let other_angle = block_type == 1 && block_mode != 1;
            if program_cells.contains(&(c + 1)) && !other_angle {
                chapters.push(t);
            }
            if other_angle {
                continue;
            }
            let first = u32_at(cp, 8).ok_or_else(bad)?;
            let last = u32_at(cp, 20).ok_or_else(bad)?;
            if last >= first {
                cells.push((first, last));
            }
            t += playback_time(&cp[4..8]);
        }
        titles.push(DvdTitle {
            number: u32::from(entry & 0x7F),
            duration_secs: if duration > 0.0 { duration } else { t },
            cells,
            chapters,
            audio,
            subtitles,
            palette: (0..16).filter_map(|k| u32_at(p, 0xA4 + 4 * k)).collect(),
            height,
        });
    }
    Ok(titles)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_titles_cells_chapters_and_languages() {
        let ifo = include_bytes!("../tests/fixtures/dvd_VTS_01_0.IFO");
        let titles = parse_vts_ifo(ifo).unwrap();
        assert_eq!(titles.len(), 2);
        let t = &titles[0];
        assert_eq!(t.number, 1);
        assert!((t.duration_secs - 30.0).abs() < 0.01);
        assert_eq!(t.cells, vec![(0, 1291), (1292, 2583)]);
        assert_eq!(t.chapters, vec![0.0, 15.0]);
        assert_eq!(t.audio.len(), 2);
        assert_eq!(t.audio[0].stream_id, 0x80);
        assert_eq!(t.audio[1].stream_id, 0x81);
        assert_eq!(t.audio[1].language.as_deref(), Some("fr"));
        assert_eq!(t.audio[0].channels, 6);
        assert_eq!(t.subtitles[0].stream_id, 0x20);
        assert_eq!(t.subtitles[0].language.as_deref(), Some("en"));
        assert_eq!(titles[1].cells, vec![(2584, 2864)]);
        assert_eq!(titles[1].sectors(), 281);
        assert!(parse_vts_ifo(b"DVDVIDEO-VMG").is_err());
    }

    #[test]
    fn bcd_times() {
        assert!(
            (playback_time(&[0x01, 0x32, 0x05, 0xC0 | 0x15]) - (5525.0 + 15.0 * 1001.0 / 30000.0))
                .abs()
                < 1e-9
        );
        assert!((playback_time(&[0x00, 0x00, 0x10, 0x40 | 0x12]) - 10.48).abs() < 1e-9);
    }
}
