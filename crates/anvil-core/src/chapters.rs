// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Chapters: read from the source, edit, generate, and import or export as
//! Matroska XML or OGM text (the two formats mkvmerge and most tools use).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    pub start_secs: f64,
    pub title: String,
}

/// What the output gets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ChapterPlan {
    /// The source's chapters, as they are.
    #[default]
    Keep,
    /// No chapters.
    Drop,
    /// An edited, generated or imported list, written to the output.
    Custom { chapters: Vec<Chapter> },
}

impl ChapterPlan {
    pub fn describe(&self, source_count: usize) -> String {
        match self {
            ChapterPlan::Keep if source_count == 0 => "None in the source".into(),
            ChapterPlan::Keep => format!("{source_count} from the source"),
            ChapterPlan::Drop => "Left out".into(),
            ChapterPlan::Custom { chapters } => format!("{} (edited)", chapters.len()),
        }
    }
}

/// Chapters from ffprobe's `-show_chapters` output.
pub fn from_ffprobe(chapters: &[Value]) -> Vec<Chapter> {
    chapters
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let start = c
                .get("start_time")
                .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or(v.as_f64()))?;
            let title = c
                .get("tags")
                .and_then(|t| t.get("title"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| default_title(i));
            Some(Chapter {
                start_secs: start,
                title,
            })
        })
        .collect()
}

pub fn default_title(i: usize) -> String {
    format!("Chapter {:02}", i + 1)
}

/// A chapter every `minutes`, starting at zero.
pub fn every(minutes: f64, duration_secs: f64) -> Result<Vec<Chapter>> {
    if minutes.is_nan() || minutes < 1.0 || duration_secs <= 0.0 {
        return Err(Error::Invalid {
            field: "chapter interval",
            reason: "use at least one minute, on a source with a known length".into(),
        });
    }
    let step = minutes * 60.0;
    let count = (duration_secs / step).ceil().max(1.0) as usize;
    Ok((0..count)
        .map(|i| Chapter {
            start_secs: i as f64 * step,
            title: default_title(i),
        })
        .collect())
}

/// Sort by time, drop duplicates and anything past the end, and make sure
/// the first chapter starts at zero.
pub fn tidy(chapters: &mut Vec<Chapter>, duration_secs: Option<f64>) {
    chapters.retain(|c| c.start_secs >= 0.0 && duration_secs.is_none_or(|d| c.start_secs < d));
    chapters.sort_by(|a, b| a.start_secs.total_cmp(&b.start_secs));
    chapters.dedup_by(|a, b| (a.start_secs - b.start_secs).abs() < 0.001);
    if let Some(first) = chapters.first_mut() {
        if first.start_secs < 1.0 {
            first.start_secs = 0.0;
        }
    }
}

/// Add a chapter at a time (kept in order). Returns its position.
pub fn add(
    chapters: &mut Vec<Chapter>,
    at_secs: f64,
    title: Option<String>,
    duration_secs: Option<f64>,
) -> Result<usize> {
    if at_secs < 0.0 || duration_secs.is_some_and(|d| at_secs >= d) {
        return Err(Error::Invalid {
            field: "chapter time",
            reason: format!("{} is outside the film", format_time(at_secs)),
        });
    }
    if chapters
        .iter()
        .any(|c| (c.start_secs - at_secs).abs() < 0.5)
    {
        return Err(Error::Invalid {
            field: "chapter time",
            reason: format!("there is already a chapter at {}", format_time(at_secs)),
        });
    }
    let pos = chapters
        .iter()
        .position(|c| c.start_secs > at_secs)
        .unwrap_or(chapters.len());
    let title = title.unwrap_or_else(|| default_title(pos));
    chapters.insert(
        pos,
        Chapter {
            start_secs: at_secs,
            title,
        },
    );
    Ok(pos)
}

/// Replace every title with "Chapter NN" in order.
pub fn renumber(chapters: &mut [Chapter]) {
    for (i, c) in chapters.iter_mut().enumerate() {
        c.title = default_title(i);
    }
}

/// "01:02:03.456".
pub fn format_time(secs: f64) -> String {
    let ms = (secs.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// "1:02:03.456", "62:03", "3723.5" or "01:02:03.456789012" to seconds.
pub fn parse_time(s: &str) -> Option<f64> {
    let s = s.trim();
    let mut total = 0.0;
    let parts: Vec<&str> = s.split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    for (i, p) in parts.iter().enumerate() {
        let v: f64 = p.trim().parse().ok().filter(|v: &f64| *v >= 0.0)?;
        if i + 1 < parts.len() && v.fract() != 0.0 {
            return None;
        }
        total = total * 60.0 + v;
    }
    Some(total)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Matroska chapter XML, as mkvmerge `--chapters` and mkvextract use.
pub fn to_matroska_xml(chapters: &[Chapter], language: &str) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- Written by Anvil -->\n<Chapters>\n  <EditionEntry>\n");
    for c in chapters {
        let ns = (c.start_secs.max(0.0) * 1e9).round() as u64;
        s.push_str(&format!(
            "    <ChapterAtom>\n      <ChapterTimeStart>{:02}:{:02}:{:02}.{:09}</ChapterTimeStart>\n      <ChapterDisplay>\n        <ChapterString>{}</ChapterString>\n        <ChapterLanguage>{}</ChapterLanguage>\n      </ChapterDisplay>\n    </ChapterAtom>\n",
            ns / 3_600_000_000_000,
            ns / 60_000_000_000 % 60,
            ns / 1_000_000_000 % 60,
            ns % 1_000_000_000,
            xml_escape(&c.title),
            xml_escape(language)
        ));
    }
    s.push_str("  </EditionEntry>\n</Chapters>\n");
    s
}

/// OGM ("simple") chapters: CHAPTER01=00:00:00.000 / CHAPTER01NAME=...
pub fn to_ogm(chapters: &[Chapter]) -> String {
    chapters
        .iter()
        .enumerate()
        .map(|(i, c)| {
            format!(
                "CHAPTER{n:02}={}\nCHAPTER{n:02}NAME={}\n",
                format_time(c.start_secs),
                c.title,
                n = i + 1
            )
        })
        .collect()
}

/// Read Matroska XML or OGM chapter text.
pub fn parse(text: &str) -> Result<Vec<Chapter>> {
    let text = text.trim_start_matches('\u{feff}');
    let chapters = if text.contains("<Chapters") {
        parse_xml(text)
    } else {
        parse_ogm(text)
    };
    let mut chapters = chapters?;
    if chapters.is_empty() {
        return Err(Error::Invalid {
            field: "chapters",
            reason: "no chapters found (expected Matroska XML or CHAPTER01=... lines)".into(),
        });
    }
    tidy(&mut chapters, None);
    Ok(chapters)
}

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let a = s.find(open)? + open.len();
    let b = s[a..].find(close)? + a;
    Some(&s[a..b])
}

fn parse_xml(text: &str) -> Result<Vec<Chapter>> {
    let mut out = Vec::new();
    for atom in text.split("<ChapterAtom>").skip(1) {
        // Nested atoms are flattened in the order they appear.
        let atom = atom.split("<ChapterAtom>").next().unwrap_or(atom);
        if between(atom, "<ChapterFlagHidden>", "</ChapterFlagHidden>")
            .is_some_and(|v| v.trim() == "1")
        {
            continue;
        }
        let start = between(atom, "<ChapterTimeStart>", "</ChapterTimeStart>")
            .and_then(parse_time)
            .ok_or(Error::Invalid {
                field: "chapters",
                reason: "a ChapterAtom has no readable ChapterTimeStart".into(),
            })?;
        let title = between(atom, "<ChapterString>", "</ChapterString>")
            .map(|t| xml_unescape(t.trim()))
            .unwrap_or_else(|| default_title(out.len()));
        out.push(Chapter {
            start_secs: start,
            title,
        });
    }
    Ok(out)
}

fn parse_ogm(text: &str) -> Result<Vec<Chapter>> {
    let mut times = std::collections::BTreeMap::<u32, f64>::new();
    let mut names = std::collections::BTreeMap::<u32, String>::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(rest) = key.trim().strip_prefix("CHAPTER") else {
            continue;
        };
        if let Some(n) = rest.strip_suffix("NAME") {
            if let Ok(n) = n.parse() {
                names.insert(n, value.trim().to_string());
            }
        } else if let Ok(n) = rest.parse::<u32>() {
            let t = parse_time(value).ok_or(Error::Invalid {
                field: "chapters",
                reason: format!("'{value}' is not a time"),
            })?;
            times.insert(n, t);
        }
    }
    Ok(times
        .into_iter()
        .enumerate()
        .map(|(i, (n, t))| Chapter {
            start_secs: t,
            title: names.remove(&n).unwrap_or_else(|| default_title(i)),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(format_time(3723.4567), "01:02:03.457");
        assert_eq!(parse_time("01:02:03.457"), Some(3723.457));
        assert_eq!(parse_time("62:03"), Some(3723.0));
        assert_eq!(parse_time("90.5"), Some(90.5));
        assert_eq!(parse_time("00:00:10.500000000"), Some(10.5));
        assert_eq!(parse_time("1:2:3:4"), None);
        assert_eq!(parse_time("1.5:00"), None);
        assert_eq!(parse_time("-3"), None);
        assert_eq!(parse_time("soon"), None);
    }

    #[test]
    fn generate_edit_and_round_trip() {
        let mut c = every(10.0, 3600.0 + 59.0).unwrap();
        assert_eq!(c.len(), 7);
        assert_eq!(c[6].start_secs, 3600.0);
        assert_eq!(c[6].title, "Chapter 07");
        assert!(every(0.5, 100.0).is_err());

        assert_eq!(
            add(
                &mut c,
                1500.0,
                Some("Heist & \"getaway\"".into()),
                Some(3659.0)
            )
            .unwrap(),
            3
        );
        assert!(
            add(&mut c, 1500.2, None, None).is_err(),
            "too close to another"
        );
        assert!(
            add(&mut c, 4000.0, None, Some(3659.0)).is_err(),
            "past the end"
        );

        let xml = to_matroska_xml(&c, "eng");
        assert!(xml.contains("<ChapterTimeStart>00:25:00.000000000</ChapterTimeStart>"));
        assert!(xml.contains("Heist &amp; &quot;getaway&quot;"));
        assert_eq!(parse(&xml).unwrap(), c);

        let ogm = to_ogm(&c);
        assert!(ogm.starts_with("CHAPTER01=00:00:00.000\nCHAPTER01NAME=Chapter 01\n"));
        let back = parse(&ogm).unwrap();
        assert_eq!(back.len(), c.len());
        assert_eq!(back[3].title, "Heist & \"getaway\"");

        renumber(&mut c);
        assert_eq!(c[3].title, "Chapter 04");
        assert!(parse("nothing here").is_err());
    }

    #[test]
    fn tidy_and_ffprobe() {
        let mut c = vec![
            Chapter {
                start_secs: 20.0,
                title: "b".into(),
            },
            Chapter {
                start_secs: 0.4,
                title: "a".into(),
            },
            Chapter {
                start_secs: 20.0,
                title: "dup".into(),
            },
            Chapter {
                start_secs: 99.0,
                title: "late".into(),
            },
        ];
        tidy(&mut c, Some(50.0));
        assert_eq!(
            c.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(c[0].start_secs, 0.0);

        let v: Value = serde_json::from_str(
            r#"[{"start_time":"0.000000","tags":{"title":"Opening"}},{"start_time":"312.5"}]"#,
        )
        .unwrap();
        let c = from_ffprobe(v.as_array().unwrap());
        assert_eq!(c[0].title, "Opening");
        assert_eq!(
            c[1],
            Chapter {
                start_secs: 312.5,
                title: "Chapter 02".into()
            }
        );
        // Hidden atoms are skipped on import.
        let hidden = "<Chapters><EditionEntry><ChapterAtom><ChapterTimeStart>00:00:00</ChapterTimeStart><ChapterFlagHidden>1</ChapterFlagHidden></ChapterAtom><ChapterAtom><ChapterTimeStart>00:01:00</ChapterTimeStart></ChapterAtom></EditionEntry></Chapters>";
        assert_eq!(
            parse(hidden).unwrap(),
            vec![Chapter {
                start_secs: 60.0,
                title: "Chapter 01".into()
            }]
        );
    }
}
