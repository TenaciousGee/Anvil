// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! OCR of image subtitles (Blu-ray PGS, DVD VobSub) to SRT with Tesseract.
//!
//! Each caption's bitmap is decoded here, turned into dark text on a white
//! page (the fill becomes ink; outlines and the transparent background
//! become paper), enlarged when the lettering is small, and written as a
//! PGM image. Tesseract then reads a batch of images in one run; its pages
//! are separated by form feeds, so captions map back by position.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::subtitles::pgs;
use crate::vobsub;

/// One caption: times (ms), whether it is forced, and its image as dark
/// text on white (one byte per pixel).
#[derive(Debug, Clone, PartialEq)]
pub struct Caption {
    pub start_ms: u64,
    pub end_ms: u64,
    pub forced: bool,
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

/// The ink (text) mask from colour and opacity: bright, mostly opaque
/// pixels are lettering; dark outlines and transparency are not.
fn ink(luma: u8, alpha: u8) -> bool {
    alpha >= 128 && luma >= 128
}

/// Paper-white page with black text, a margin, and lettering enlarged
/// when it is small (DVD captions are about 20 pixels high).
fn page(width: usize, height: usize, mask: &[bool]) -> (usize, usize, Vec<u8>) {
    let rows: Vec<usize> = (0..height)
        .filter(|y| mask[y * width..(y + 1) * width].iter().any(|m| *m))
        .collect();
    let text_h = rows.last().map_or(0, |l| l - rows[0] + 1);
    let scale = if text_h > 0 && text_h < 40 { 2 } else { 1 };
    let margin = 12;
    let (w, h) = (width * scale + 2 * margin, height * scale + 2 * margin);
    let mut px = vec![255u8; w * h];
    for y in 0..height * scale {
        for x in 0..width * scale {
            if mask[(y / scale) * width + x / scale] {
                px[(y + margin) * w + x + margin] = 0;
            }
        }
    }
    (w, h, px)
}

/// Captions from a PGS stream: each display set with objects starts one,
/// and the next display set ends it.
pub fn pgs_captions(data: &[u8]) -> Result<Vec<Caption>, String> {
    use std::collections::HashMap;
    let segs = pgs::segments(data)?;
    let mut palettes: HashMap<u8, [(u8, u8); 256]> = HashMap::new();
    let mut objects: HashMap<u16, Vec<u8>> = HashMap::new();
    let mut out: Vec<Caption> = Vec::new();
    let mut open: Option<usize> = None;
    let ms = |pts: u32| u64::from(pts) / 90;
    for s in &segs {
        match s.kind {
            pgs::PDS if s.payload.len() >= 2 => {
                let pal = palettes.entry(s.payload[0]).or_insert([(0, 0); 256]);
                for e in s.payload[2..].chunks_exact(5) {
                    pal[usize::from(e[0])] = (e[1], e[4]);
                }
            }
            pgs::ODS if s.payload.len() >= 4 => {
                let id = u16::from_be_bytes([s.payload[0], s.payload[1]]);
                let first = s.payload[3] & 0x80 != 0;
                let body = if first {
                    s.payload.get(7..).unwrap_or(&[])
                } else {
                    s.payload.get(4..).unwrap_or(&[])
                };
                let e = objects.entry(id).or_default();
                if first {
                    e.clear();
                }
                e.extend_from_slice(body);
            }
            pgs::END => {}
            pgs::PCS => {
                // Close whatever is on screen.
                if let Some(i) = open.take() {
                    out[i].end_ms = ms(s.pts);
                }
            }
            _ => {}
        }
        // Compose at the end of each display set.
        if s.kind != pgs::END {
            continue;
        }
        let Some(pcs) = segs
            .iter()
            .rev()
            .find(|p| p.kind == pgs::PCS && p.pts <= s.pts)
        else {
            continue;
        };
        if pcs.pts != s.pts {
            continue;
        }
        let objs = pgs::objects(&pcs.payload);
        if objs.is_empty() {
            continue;
        }
        let palette = palettes
            .get(&pcs.payload.get(9).copied().unwrap_or(0))
            .copied()
            .unwrap_or([(0, 0); 256]);
        // Canvas covering all objects.
        let mut placed = Vec::new();
        for o in &objs {
            let Some(bmp) = objects.get(&o.id) else {
                continue;
            };
            if bmp.len() < 4 {
                continue;
            }
            let w = usize::from(u16::from_be_bytes([bmp[0], bmp[1]]));
            let h = usize::from(u16::from_be_bytes([bmp[2], bmp[3]]));
            let x = usize::from(u16::from_be_bytes([o.rest[0], o.rest[1]]));
            let y = usize::from(u16::from_be_bytes([o.rest[2], o.rest[3]]));
            placed.push((
                x,
                y,
                w,
                h,
                decode_pgs_rle(&bmp[4..], w, h),
                o.flags & pgs::FORCED != 0,
            ));
        }
        if placed.is_empty() {
            continue;
        }
        let x0 = placed.iter().map(|p| p.0).min().unwrap_or(0);
        let y0 = placed.iter().map(|p| p.1).min().unwrap_or(0);
        let x1 = placed.iter().map(|p| p.0 + p.2).max().unwrap_or(0);
        let y1 = placed.iter().map(|p| p.1 + p.3).max().unwrap_or(0);
        let (cw, ch) = (x1 - x0, y1 - y0);
        let mut mask = vec![false; cw * ch];
        for (x, y, w, h, idx, _) in &placed {
            for row in 0..*h {
                for col in 0..*w {
                    let (luma, alpha) = palette[usize::from(idx[row * w + col])];
                    if ink(luma, alpha) {
                        mask[(y - y0 + row) * cw + (x - x0 + col)] = true;
                    }
                }
            }
        }
        let (w, h, pixels) = page(cw, ch, &mask);
        out.push(Caption {
            start_ms: ms(pcs.pts),
            end_ms: ms(pcs.pts) + 5000,
            forced: placed.iter().any(|p| p.5),
            width: w,
            height: h,
            pixels,
        });
        open = Some(out.len() - 1);
    }
    Ok(out)
}

/// PGS run-length bitmap to palette indices.
fn decode_pgs_rle(d: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut px = vec![0u8; w * h];
    let (mut i, mut x, mut y) = (0, 0, 0);
    while i < d.len() && y < h {
        let b = d[i];
        i += 1;
        let (colour, run) = if b != 0 {
            (b, 1)
        } else {
            let Some(&f) = d.get(i) else { break };
            i += 1;
            if f == 0 {
                // End of line.
                x = 0;
                y += 1;
                continue;
            }
            let long = f & 0x40 != 0;
            let mut run = usize::from(f & 0x3F);
            if long {
                run = run << 8 | usize::from(*d.get(i).unwrap_or(&0));
                i += 1;
            }
            let colour = if f & 0x80 != 0 {
                let c = *d.get(i).unwrap_or(&0);
                i += 1;
                c
            } else {
                0
            };
            (colour, run)
        };
        for _ in 0..run {
            if x < w {
                px[y * w + x] = colour;
            }
            x += 1;
        }
    }
    px
}

/// Captions from a VobSub pair.
pub fn vobsub_captions(f: &vobsub::VobSubFile) -> Vec<Caption> {
    let mut out: Vec<Caption> = Vec::new();
    for (k, (ms, data)) in f.spus.iter().enumerate() {
        let Some(img) = vobsub::decode_spu(data) else {
            continue;
        };
        let (w, h) = (img.width as usize, img.height as usize);
        let mask: Vec<bool> = img
            .pixels
            .iter()
            .map(|p| {
                let c = f
                    .palette
                    .get(usize::from(img.colours[usize::from(*p)]))
                    .copied()
                    .unwrap_or([0, 0, 0]);
                let luma = (0.299 * f64::from(c[0])
                    + 0.587 * f64::from(c[1])
                    + 0.114 * f64::from(c[2])) as u8;
                ink(luma, img.alpha[usize::from(*p)] * 17)
            })
            .collect();
        let (pw, ph, pixels) = page(w, h, &mask);
        let start = ms + img.start_ms;
        let next = f.spus.get(k + 1).map(|(m, _)| *m);
        // A stop a minute or more away (0xFFFF, "never", from some
        // authoring tools) is no stop at all: the next caption ends it.
        let end = img
            .stop_ms
            .filter(|s| *s < 60_000)
            .map(|s| ms + s)
            .or(next)
            .unwrap_or(start + 5000)
            .max(start + 200);
        out.push(Caption {
            start_ms: start,
            end_ms: end,
            forced: img.forced,
            width: pw,
            height: ph,
            pixels,
        });
    }
    out
}

/// Tesseract's name for a subtitle language (ISO 639-1 or -2).
pub fn tesseract_language(lang: Option<&str>) -> &'static str {
    match lang.unwrap_or("eng").to_ascii_lowercase().as_str() {
        "fre" | "fra" | "fr" => "fra",
        "ger" | "deu" | "de" => "deu",
        "spa" | "es" => "spa",
        "ita" | "it" => "ita",
        "dut" | "nld" | "nl" => "nld",
        "por" | "pt" => "por",
        "swe" | "sv" => "swe",
        "dan" | "da" => "dan",
        "nor" | "nob" | "no" | "nb" => "nor",
        "fin" | "fi" => "fin",
        "pol" | "pl" => "pol",
        "cze" | "ces" | "cs" => "ces",
        "hun" | "hu" => "hun",
        "gre" | "ell" | "el" => "ell",
        "tur" | "tr" => "tur",
        "rus" | "ru" => "rus",
        "ukr" | "uk" => "ukr",
        "ara" | "ar" => "ara",
        "heb" | "he" => "heb",
        "hin" | "hi" => "hin",
        "tha" | "th" => "tha",
        "vie" | "vi" => "vie",
        "jpn" | "ja" => "jpn",
        "kor" | "ko" => "kor",
        "chi" | "zho" | "zh" => "chi_sim",
        _ => "eng",
    }
}

/// Languages Tesseract has data for.
pub fn installed_languages() -> Vec<String> {
    crate::process::run_with_timeout(
        "tesseract",
        &["--list-langs".to_string()],
        Duration::from_secs(20),
    )
    .map(|o| {
        format!("{}\n{}", o.stdout, o.stderr)
            .lines()
            .skip_while(|l| !l.starts_with("List of available"))
            .skip(1)
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

fn srt_time(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// Common OCR slips in subtitles.
pub fn tidy_text(t: &str) -> String {
    t.lines()
        .map(|l| {
            let l = l
                .trim()
                .replace('|', "I")
                .replace("’", "'")
                .replace("‘", "'");
            // A lone "l" or "1" where "I" was meant ("l was there").
            let l = if l.starts_with("l ") {
                format!("I{}", &l[1..])
            } else {
                l
            };
            l.replace(" l ", " I ")
        })
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn to_srt(entries: &[(u64, u64, String)]) -> String {
    let mut s = String::new();
    let mut n = 0;
    for (start, end, text) in entries.iter().filter(|e| !e.2.is_empty()) {
        n += 1;
        s.push_str(&format!(
            "{n}\n{} --> {}\n{text}\n\n",
            srt_time(*start),
            srt_time(*end)
        ));
    }
    s
}

/// Read captions with Tesseract in batches, reporting progress (0 to 1).
/// Returns (start, end, text) for each caption, in order.
pub fn recognise(
    captions: &[Caption],
    language: &str,
    work: &Path,
    cancel: &dyn Fn() -> bool,
    mut on_progress: impl FnMut(f64),
) -> Result<Vec<(u64, u64, String)>, String> {
    let installed = installed_languages();
    if installed.is_empty() {
        return Err(
            "Tesseract is not installed (on CachyOS: sudo pacman -S tesseract tesseract-data-eng)"
                .into(),
        );
    }
    if !installed.iter().any(|l| l == language) {
        return Err(format!(
            "Tesseract has no '{language}' language data (on CachyOS: sudo pacman -S tesseract-data-{language})"
        ));
    }
    std::fs::create_dir_all(work).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(captions.len());
    for (b, batch) in captions.chunks(100).enumerate() {
        if cancel() {
            return Err("cancelled".into());
        }
        let mut list = String::new();
        let mut files: Vec<PathBuf> = Vec::new();
        for (k, c) in batch.iter().enumerate() {
            let p = work.join(format!("caption_{:05}.pgm", b * 100 + k));
            let mut img = format!("P5\n{} {}\n255\n", c.width, c.height).into_bytes();
            img.extend_from_slice(&c.pixels);
            std::fs::write(&p, img).map_err(|e| e.to_string())?;
            list.push_str(&format!("{}\n", p.display()));
            files.push(p);
        }
        let list_path = work.join("captions.txt");
        std::fs::write(&list_path, list).map_err(|e| e.to_string())?;
        let o = crate::process::run_with_timeout(
            "tesseract",
            &[
                list_path.to_string_lossy().into_owned(),
                "stdout".into(),
                "-l".into(),
                language.into(),
                "--psm".into(),
                "6".into(),
            ],
            Duration::from_secs(600),
        )
        .map_err(|e| e.to_string())?;
        let pages: Vec<&str> = o.stdout.split('\u{c}').collect();
        for (k, c) in batch.iter().enumerate() {
            out.push((
                c.start_ms,
                c.end_ms,
                tidy_text(pages.get(k).copied().unwrap_or("")),
            ));
        }
        for f in files {
            let _ = std::fs::remove_file(f);
        }
        on_progress(((b + 1) * 100).min(captions.len()) as f64 / captions.len().max(1) as f64);
    }
    Ok(out)
}

/// OCR a PGS (`.sup`) or VobSub (`.idx` with `.sub`) file to SRT text.
/// Returns the SRT and the number of captions read.
pub fn file_to_srt(
    input: &Path,
    language: Option<&str>,
    forced_only: bool,
    work: &Path,
    cancel: &dyn Fn() -> bool,
    on_progress: impl FnMut(f64),
) -> Result<(String, usize), String> {
    let ext = input
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut captions = match ext.as_str() {
        "sup" => {
            pgs_captions(&std::fs::read(input).map_err(|e| format!("{}: {e}", input.display()))?)?
        }
        "idx" => vobsub_captions(&vobsub::read_idx_sub(input)?),
        _ => {
            return Err(format!(
                "{} is not a PGS (.sup) or VobSub (.idx) file",
                input.display()
            ))
        }
    };
    if forced_only {
        captions.retain(|c| c.forced);
    }
    let entries = recognise(
        &captions,
        tesseract_language(language),
        work,
        cancel,
        on_progress,
    )?;
    let read = entries.iter().filter(|e| !e.2.is_empty()).count();
    Ok((to_srt(&entries), read))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_and_text() {
        assert_eq!(tesseract_language(Some("fre")), "fra");
        assert_eq!(tesseract_language(Some("de")), "deu");
        assert_eq!(tesseract_language(None), "eng");
        assert_eq!(tesseract_language(Some("xx")), "eng");
        assert_eq!(
            tidy_text("  l was there |  \n\n Hello  "),
            "I was there I\nHello"
        );
        let srt = to_srt(&[
            (1000, 3500, "Hello".into()),
            (4000, 4500, String::new()),
            (5000, 7500, "Two\nlines".into()),
        ]);
        assert_eq!(srt, "1\n00:00:01,000 --> 00:00:03,500\nHello\n\n2\n00:00:05,000 --> 00:00:07,500\nTwo\nlines\n\n");
    }

    #[test]
    fn pgs_and_vobsub_bitmaps() {
        // PGS: a forced caption at 3 s cleared at 4 s.
        let data = pgs::build(&[
            (3.0, 0, 0x80, &[(1, true)], true),
            (4.0, 1, 0x00, &[], false),
        ]);
        let caps = pgs_captions(&data).unwrap();
        assert_eq!(caps.len(), 1);
        assert_eq!(
            (caps[0].start_ms, caps[0].end_ms, caps[0].forced),
            (3000, 4000, true)
        );
        assert_eq!(
            decode_pgs_rle(&[0x01, 0x00, 0x83, 0x02, 0x00, 0x00], 4, 1),
            vec![1, 2, 2, 2]
        );

        // VobSub: timing from the unit's own start and stop delays.
        let f = vobsub::VobSubFile {
            size: (720, 480),
            palette: vec![[0, 0, 0], [235, 235, 235], [16, 16, 16], [128, 128, 128]],
            spus: vec![(4000, vobsub::tests_support::make_spu(40, 10, 88, 264))],
        };
        let caps = vobsub_captions(&f);
        assert_eq!(caps.len(), 1);
        // 88 * 1024 / 90 = 1001 ms, 264 * 1024 / 90 = 3003 ms.
        assert_eq!((caps[0].start_ms, caps[0].end_ms), (5001, 7003));
        // Enlarged (a 10-pixel caption) with a 12-pixel margin each side.
        assert_eq!((caps[0].width, caps[0].height), (40 * 2 + 24, 10 * 2 + 24));
    }
}
