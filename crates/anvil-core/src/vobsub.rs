// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! DVD subtitles (VobSub): pulling subpicture units out of a VOB, writing
//! them as an `.idx`/`.sub` pair, reading such pairs back, and decoding a
//! subpicture's bitmap (for OCR).
//!
//! A DVD carries subtitles as subpicture units (SPUs) inside private
//! stream 1 packets (sub-stream 0x20 + n) of the MPEG program stream. Each
//! SPU holds a two-bit run-length bitmap and a control sequence with its
//! position, colours and display times. The colours index the title's
//! 16-colour palette from the IFO, which is written into the `.idx`.

use std::io::{Read, Write};
use std::path::Path;

/// One subpicture unit and its presentation time (90 kHz).
#[derive(Debug, Clone, PartialEq)]
pub struct Spu {
    pub pts: u64,
    pub data: Vec<u8>,
}

/// Subpicture streams found in a program stream, by sub-stream number
/// (0 for 0x20), plus the first video timestamp (the title's start).
#[derive(Debug, Default)]
pub struct Extracted {
    pub streams: std::collections::BTreeMap<u8, Vec<Spu>>,
    pub first_video_pts: Option<u64>,
}

fn read_pts(b: &[u8]) -> Option<u64> {
    if b.len() < 5 {
        return None;
    }
    Some(
        (u64::from(b[0] >> 1) & 7) << 30
            | u64::from(b[1]) << 22
            | (u64::from(b[2]) >> 1) << 15
            | u64::from(b[3]) << 7
            | u64::from(b[4]) >> 1,
    )
}

/// Walk a program stream and collect subpicture units. Packets are found by
/// start code, so pack alignment does not matter. The stream is read in
/// chunks, so a title of several gigabytes needs little memory.
pub fn extract(ps: &mut impl Read) -> std::io::Result<Extracted> {
    let mut out = Extracted::default();
    // SPUs being assembled: (pts, expected size, data).
    let mut open: std::collections::BTreeMap<u8, (u64, usize, Vec<u8>)> = Default::default();
    let mut data: Vec<u8> = Vec::with_capacity(8 << 20);
    let mut chunk = vec![0u8; 4 << 20];
    let mut eof = false;
    let mut i = 0;
    loop {
        // Keep at least one whole packet (6 + 65535 bytes) ahead, or all of it.
        if !eof && data.len() - i < 70_000 {
            data.drain(..i);
            i = 0;
            let n = ps.read(&mut chunk)?;
            if n == 0 {
                eof = true;
            } else {
                data.extend_from_slice(&chunk[..n]);
                continue;
            }
        }
        if i + 4 > data.len() {
            break;
        }
        if data[i] != 0 || data[i + 1] != 0 || data[i + 2] != 1 {
            i += 1;
            continue;
        }
        let id = data[i + 3];
        match id {
            0xBA => {
                // MPEG-2 pack header: 14 bytes plus stuffing.
                let stuffing = data.get(i + 13).map_or(0, |b| usize::from(b & 7));
                i += 14 + stuffing;
                continue;
            }
            0xB9 => break,
            _ => {}
        }
        let Some(len) = data
            .get(i + 4..i + 6)
            .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
        else {
            break;
        };
        let start = i + 6;
        let end = (start + len).min(data.len());
        let pes = &data[start..end];
        let is_video = (0xE0..=0xEF).contains(&id);
        if (id == 0xBD || is_video) && pes.len() >= 3 && pes[0] & 0xC0 == 0x80 {
            let has_pts = pes[1] & 0x80 != 0;
            let header_len = usize::from(pes[2]);
            let pts = if has_pts { read_pts(&pes[3..]) } else { None };
            if is_video {
                if out.first_video_pts.is_none() {
                    out.first_video_pts = pts;
                }
            } else if let Some(&sub) = pes.get(3 + header_len) {
                if (0x20..0x40).contains(&sub) {
                    let n = sub - 0x20;
                    let payload = &pes[(4 + header_len).min(pes.len())..];
                    if let Some(pts) = pts {
                        // A new unit starts; its first two bytes are its size.
                        if payload.len() >= 2 {
                            let size = usize::from(u16::from_be_bytes([payload[0], payload[1]]));
                            open.insert(n, (pts, size, payload.to_vec()));
                        }
                    } else if let Some((_, _, buf)) = open.get_mut(&n) {
                        buf.extend_from_slice(payload);
                    }
                    if let Some((pts, size, buf)) = open.get(&n) {
                        if buf.len() >= *size && *size > 4 {
                            let spu = Spu {
                                pts: *pts,
                                data: buf[..*size].to_vec(),
                            };
                            out.streams.entry(n).or_default().push(spu);
                            open.remove(&n);
                        }
                    }
                }
            }
        }
        i = end;
    }
    Ok(out)
}

/// 0x00YYCrCb to "rrggbb".
pub fn ycrcb_to_rgb(v: u32) -> String {
    let y = f64::from((v >> 16) & 0xFF);
    let cr = f64::from((v >> 8) & 0xFF) - 128.0;
    let cb = f64::from(v & 0xFF) - 128.0;
    let c = |x: f64| x.round().clamp(0.0, 255.0) as u8;
    format!(
        "{:02x}{:02x}{:02x}",
        c(y + 1.402 * cr),
        c(y - 0.344_136 * cb - 0.714_136 * cr),
        c(y + 1.772 * cb)
    )
}

fn timestamp(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}:{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

fn pes_pts(pts: u64) -> [u8; 5] {
    [
        0x21 | ((pts >> 29) as u8 & 0x0E),
        (pts >> 22) as u8,
        0x01 | ((pts >> 14) as u8 & 0xFE),
        (pts >> 7) as u8,
        0x01 | ((pts << 1) as u8 & 0xFE),
    ]
}

fn pack_header(scr: u64) -> [u8; 14] {
    [
        0x00,
        0x00,
        0x01,
        0xBA,
        0x44 | ((scr >> 27) as u8 & 0x38) | ((scr >> 28) as u8 & 0x03),
        (scr >> 20) as u8,
        0x04 | ((scr >> 12) as u8 & 0xF8) | ((scr >> 13) as u8 & 0x03),
        (scr >> 5) as u8,
        0x04 | ((scr << 3) as u8 & 0xF8),
        0x01,
        0x01,
        0x89,
        0xC3,
        0xF8,
    ]
}

/// Write one stream as a VobSub pair (`path.idx` and `path.sub`). Times in
/// the index are relative to `base_pts` (the title's first video frame).
pub fn write_idx_sub(
    spus: &[Spu],
    base_pts: u64,
    palette: &[u32],
    size: (u32, u32),
    language: Option<&str>,
    idx_path: &Path,
) -> std::io::Result<()> {
    let mut sub = Vec::new();
    let mut idx = String::from("# VobSub index file, v7 (do not modify this line!)\n# Written by Anvil from a DVD title.\n");
    idx.push_str(&format!("size: {}x{}\n", size.0, size.1));
    let colours: Vec<String> = (0..16)
        .map(|k| ycrcb_to_rgb(palette.get(k).copied().unwrap_or(0x0010_8080)))
        .collect();
    idx.push_str(&format!("palette: {}\n\n", colours.join(", ")));
    let lang = language.filter(|l| l.len() == 2).unwrap_or("en");
    idx.push_str(&format!("id: {lang}, index: 0\n"));
    for spu in spus {
        let pts = spu.pts.saturating_sub(base_pts);
        idx.push_str(&format!(
            "timestamp: {}, filepos: {:09x}\n",
            timestamp(pts / 90),
            sub.len()
        ));
        // One pack per chunk, the first carrying the timestamp.
        for (k, chunk) in spu.data.chunks(2000).enumerate() {
            sub.extend_from_slice(&pack_header(spu.pts));
            let mut pes = vec![0x81];
            if k == 0 {
                pes.extend_from_slice(&[0x80, 5]);
                pes.extend_from_slice(&pes_pts(spu.pts));
            } else {
                pes.extend_from_slice(&[0x00, 0]);
            }
            pes.push(0x20);
            pes.extend_from_slice(chunk);
            sub.extend_from_slice(&[0, 0, 1, 0xBD]);
            sub.extend_from_slice(&(pes.len() as u16).to_be_bytes());
            sub.extend_from_slice(&pes);
        }
    }
    std::fs::File::create(idx_path.with_extension("sub"))?.write_all(&sub)?;
    std::fs::write(idx_path, idx)
}

/// A VobSub pair read back: size, palette (RGB) and units with their times
/// (ms from the start).
#[derive(Debug, Default)]
pub struct VobSubFile {
    pub size: (u32, u32),
    pub palette: Vec<[u8; 3]>,
    pub spus: Vec<(u64, Vec<u8>)>,
}

/// Read an `.idx`/`.sub` pair (the first stream listed).
pub fn read_idx_sub(idx_path: &Path) -> Result<VobSubFile, String> {
    let idx =
        std::fs::read_to_string(idx_path).map_err(|e| format!("{}: {e}", idx_path.display()))?;
    let sub_path = idx_path.with_extension("sub");
    let sub = std::fs::read(&sub_path).map_err(|e| format!("{}: {e}", sub_path.display()))?;
    let mut f = VobSubFile::default();
    let mut entries = Vec::new();
    let mut streams = 0;
    for line in idx.lines().map(str::trim) {
        if let Some(s) = line.strip_prefix("size:") {
            let (w, h) = s.trim().split_once('x').unwrap_or(("720", "480"));
            f.size = (
                w.trim().parse().unwrap_or(720),
                h.trim().parse().unwrap_or(480),
            );
        } else if let Some(p) = line.strip_prefix("palette:") {
            f.palette = p
                .split(',')
                .filter_map(|c| u32::from_str_radix(c.trim(), 16).ok())
                .map(|v| [(v >> 16) as u8, (v >> 8) as u8, v as u8])
                .collect();
        } else if line.starts_with("id:") {
            streams += 1;
        } else if let (1, Some(rest)) = (streams, line.strip_prefix("timestamp:")) {
            let (ts, pos) = rest.split_once(", filepos:").ok_or("bad timestamp line")?;
            let parts: Vec<u64> = ts
                .trim()
                .split(':')
                .filter_map(|p| p.parse().ok())
                .collect();
            if parts.len() == 4 {
                let ms = parts[0] * 3_600_000 + parts[1] * 60_000 + parts[2] * 1000 + parts[3];
                let pos = usize::from_str_radix(pos.trim(), 16).map_err(|e| e.to_string())?;
                entries.push((ms, pos));
            }
        }
    }
    for (ms, pos) in entries {
        let Some(slice) = sub.get(pos..) else {
            continue;
        };
        let ex = extract(&mut &slice[..slice.len().min(1 << 16)]).map_err(|e| e.to_string())?;
        if let Some(spu) = ex.streams.values().next().and_then(|v| v.first()) {
            f.spus.push((ms, spu.data.clone()));
        }
    }
    Ok(f)
}

/// A decoded subpicture: an indexed bitmap (values 0 to 3), its four
/// palette entries and transparency, position and display times (ms from
/// the unit's timestamp).
#[derive(Debug, Clone, PartialEq)]
pub struct SpuImage {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub colours: [u8; 4],
    pub alpha: [u8; 4],
    pub start_ms: u64,
    pub stop_ms: Option<u64>,
    pub forced: bool,
}

/// Decode a subpicture unit.
pub fn decode_spu(d: &[u8]) -> Option<SpuImage> {
    let size = usize::from(u16::from_be_bytes([*d.first()?, *d.get(1)?]));
    let ctrl = usize::from(u16::from_be_bytes([*d.get(2)?, *d.get(3)?]));
    let d = d.get(..size.min(d.len()))?;
    let mut img = SpuImage {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        pixels: vec![],
        colours: [0, 1, 2, 3],
        alpha: [0, 15, 15, 15],
        start_ms: 0,
        stop_ms: None,
        forced: false,
    };
    let (mut top, mut bottom) = (0usize, 0usize);
    let mut seq = ctrl;
    // Walk the control sequences (each points to the next; the last to itself).
    for _ in 0..16 {
        let delay = u64::from(u16::from_be_bytes([*d.get(seq)?, *d.get(seq + 1)?]));
        let next = usize::from(u16::from_be_bytes([*d.get(seq + 2)?, *d.get(seq + 3)?]));
        let ms = delay * 1024 / 90;
        let mut p = seq + 4;
        loop {
            let cmd = *d.get(p)?;
            p += 1;
            match cmd {
                0x00 => {
                    img.forced = true;
                    img.start_ms = ms;
                }
                0x01 => img.start_ms = ms,
                0x02 => img.stop_ms = Some(ms),
                0x03 => {
                    let (a, b) = (*d.get(p)?, *d.get(p + 1)?);
                    img.colours = [b & 0xF, b >> 4, a & 0xF, a >> 4];
                    p += 2;
                }
                0x04 => {
                    let (a, b) = (*d.get(p)?, *d.get(p + 1)?);
                    img.alpha = [b & 0xF, b >> 4, a & 0xF, a >> 4];
                    p += 2;
                }
                0x05 => {
                    let c = d.get(p..p + 6)?;
                    let x1 = u32::from(c[0]) << 4 | u32::from(c[1]) >> 4;
                    let x2 = (u32::from(c[1]) & 0xF) << 8 | u32::from(c[2]);
                    let y1 = u32::from(c[3]) << 4 | u32::from(c[4]) >> 4;
                    let y2 = (u32::from(c[4]) & 0xF) << 8 | u32::from(c[5]);
                    img.x = x1;
                    img.y = y1;
                    img.width = (x2 + 1).saturating_sub(x1);
                    img.height = (y2 + 1).saturating_sub(y1);
                    p += 6;
                }
                0x06 => {
                    top = usize::from(u16::from_be_bytes([*d.get(p)?, *d.get(p + 1)?]));
                    bottom = usize::from(u16::from_be_bytes([*d.get(p + 2)?, *d.get(p + 3)?]));
                    p += 4;
                }
                0xFF => break,
                _ => return None,
            }
        }
        if next == seq {
            break;
        }
        seq = next;
    }
    if img.width == 0 || img.height == 0 || img.width > 2048 || img.height > 2048 {
        return None;
    }
    let (w, h) = (img.width as usize, img.height as usize);
    img.pixels = vec![0; w * h];
    // Interlaced: even lines from the top field, odd from the bottom.
    for (field, start) in [(0usize, top), (1, bottom)] {
        let mut nib = start * 2;
        let get = |n: usize| -> u8 {
            let b = d.get(n / 2).copied().unwrap_or(0);
            if n % 2 == 0 {
                b >> 4
            } else {
                b & 0xF
            }
        };
        let mut row = field;
        while row < h {
            let mut col = 0;
            while col < w {
                // Variable-length code of 4, 8, 12 or 16 bits: complete once
                // the value reaches 0x4, 0x10 or 0x40 respectively.
                let mut v = u32::from(get(nib));
                nib += 1;
                let mut bits = 4;
                while v < 0x4 << ((bits - 4) / 2) && bits < 16 {
                    v = v << 4 | u32::from(get(nib));
                    nib += 1;
                    bits += 4;
                }
                let colour = (v & 3) as u8;
                let mut run = (v >> 2) as usize;
                if run == 0 {
                    run = w - col; // to the end of the line
                }
                let run = run.min(w - col);
                img.pixels[row * w + col..row * w + col + run].fill(colour);
                col += run;
            }
            // Lines start on a byte boundary.
            if nib % 2 == 1 {
                nib += 1;
            }
            row += 2;
        }
    }
    Some(img)
}

/// Test helpers shared with the OCR tests.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// A subpicture unit: a w x h block of colour 1 inside a border of 2.
    pub(crate) fn make_spu(w: usize, h: usize, start: u16, stop: u16) -> Vec<u8> {
        let mut nibbles: Vec<Vec<u8>> = vec![Vec::new(), Vec::new()];
        let code = |n: &mut Vec<u8>, mut run: usize, colour: u8| {
            while run > 0 {
                let r = run.min(255);
                run -= r;
                let v = (r as u32) << 2 | u32::from(colour);
                let len = if r < 4 {
                    1
                } else if r < 16 {
                    2
                } else if r < 64 {
                    3
                } else {
                    4
                };
                for k in (0..len).rev() {
                    n.push(((v >> (4 * k)) & 0xF) as u8);
                }
            }
        };
        for row in 0..h {
            let n = &mut nibbles[row % 2];
            if row == 0 || row == h - 1 {
                code(n, w, 2);
            } else {
                code(n, 1, 2);
                code(n, w - 2, 1);
                code(n, 1, 2);
            }
            if n.len() % 2 == 1 {
                n.push(0);
            }
        }
        let pack = |n: &[u8]| {
            n.chunks(2)
                .map(|c| c[0] << 4 | c.get(1).copied().unwrap_or(0))
                .collect::<Vec<u8>>()
        };
        let (top, bottom) = (pack(&nibbles[0]), pack(&nibbles[1]));
        let top_off = 4u16;
        let bottom_off = top_off + top.len() as u16;
        let ctrl = bottom_off + bottom.len() as u16;
        let second = ctrl + 4 + 1 + 3 + 3 + 7 + 5 + 1;
        let mut c = Vec::new();
        c.extend_from_slice(&start.to_be_bytes());
        c.extend_from_slice(&second.to_be_bytes());
        c.push(0x01);
        c.extend_from_slice(&[0x03, 0x01, 0x23]);
        c.extend_from_slice(&[0x04, 0xFF, 0xF0]);
        let (x1, x2, y1, y2) = (10u32, 10 + w as u32 - 1, 20u32, 20 + h as u32 - 1);
        c.extend_from_slice(&[
            0x05,
            (x1 >> 4) as u8,
            ((x1 & 0xF) << 4 | x2 >> 8) as u8,
            x2 as u8,
            (y1 >> 4) as u8,
            ((y1 & 0xF) << 4 | y2 >> 8) as u8,
            y2 as u8,
        ]);
        c.extend_from_slice(&[0x06]);
        c.extend_from_slice(&top_off.to_be_bytes());
        c.extend_from_slice(&bottom_off.to_be_bytes());
        c.push(0xFF);
        c.extend_from_slice(&stop.to_be_bytes());
        c.extend_from_slice(&second.to_be_bytes());
        c.extend_from_slice(&[0x02, 0xFF]);
        let mut spu = Vec::new();
        let size = 4 + top.len() + bottom.len() + c.len();
        spu.extend_from_slice(&(size as u16).to_be_bytes());
        spu.extend_from_slice(&ctrl.to_be_bytes());
        spu.extend_from_slice(&top);
        spu.extend_from_slice(&bottom);
        spu.extend_from_slice(&c);
        spu
    }

    #[test]
    fn decode_a_subpicture() {
        let spu = make_spu(40, 10, 0, 176); // stop after 176 * 1024 / 90 ms
        let img = decode_spu(&spu).unwrap();
        assert_eq!((img.x, img.y, img.width, img.height), (10, 20, 40, 10));
        assert_eq!(img.stop_ms, Some(2002));
        assert_eq!(img.colours, [3, 2, 1, 0]);
        assert_eq!(img.alpha, [0, 15, 15, 15]);
        assert_eq!(img.pixels[0], 2);
        assert_eq!(img.pixels[40 * 5 + 20], 1, "inside the border");
        assert_eq!(img.pixels[40 * 5 + 39], 2, "right border");
        assert!(img.pixels.iter().all(|p| *p == 1 || *p == 2));
    }

    #[test]
    fn idx_sub_round_trip() {
        let dir = std::env::temp_dir().join(format!("anvil-vobsub-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let spus = vec![
            Spu {
                pts: 90_000 + 45_000,
                data: make_spu(40, 10, 0, 100),
            },
            // Big enough to need three packs (transport only, not decoded).
            Spu {
                pts: 90_000 + 450_000,
                data: {
                    let mut d: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
                    d[..2].copy_from_slice(&5000u16.to_be_bytes());
                    d
                },
            },
        ];
        assert!(spus[1].data.len() > 2000);
        let mut palette = vec![0x0010_8080; 16];
        palette[1] = 0x00EB_8080; // white
        let idx = dir.join("t.idx");
        write_idx_sub(&spus, 90_000, &palette, (720, 480), Some("fr"), &idx).unwrap();
        let text = std::fs::read_to_string(&idx).unwrap();
        assert!(text.contains("size: 720x480"));
        assert!(text.contains("palette: 101010, ebebeb, 101010"));
        assert!(text.contains("id: fr, index: 0"));
        assert!(text.contains("timestamp: 00:00:00:500, filepos: 000000000"));
        assert!(text.contains("timestamp: 00:00:05:000"));

        let back = read_idx_sub(&idx).unwrap();
        assert_eq!(back.size, (720, 480));
        assert_eq!(back.palette[1], [0xEB, 0xEB, 0xEB]);
        assert_eq!(back.spus.len(), 2);
        assert_eq!(back.spus[0], (500, spus[0].data.clone()));
        assert_eq!(back.spus[1].1, spus[1].data, "reassembled across packs");
        // Studio-range red, converted without range expansion (as VobSub tools do).
        assert_eq!(ycrcb_to_rgb(0x0051_F05A), "ee0e0e");
    }
}
