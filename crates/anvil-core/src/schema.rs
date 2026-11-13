// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Encoder option pages, driven by per-encoder schema files
//! (`schemas/<encoder>.toml`).
//!
//! A schema lists the options worth a control (number, choice, on/off,
//! text) with their ranges, defaults and a line of help. The values live in
//! the preset's `extra_args` for that encoder, so a hand-written argument
//! list and the option page always agree: the page parses the list into
//! known options plus "other arguments", and writes it back in schema
//! order. Options Anvil sets itself (rate control, colour, HDR, Dolby
//! Vision) are deliberately not in the schemas.
//!
//! Each tool's `--help` is compared with its schema (`anvil options
//! --check`, and a test against saved help texts in `schemas/help/`), so a
//! renamed or removed option in a new tool version is caught.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::encoder::EncoderKind;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptKind {
    Int,
    Float,
    Choice,
    /// `--x` on, `--no-x` (the `off` flag) off.
    Toggle,
    /// A switch with no value; present or not.
    Flag,
    Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Opt {
    pub flag: String,
    #[serde(default)]
    pub off: Option<String>,
    pub label: String,
    pub group: String,
    pub kind: OptKind,
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub pattern: Option<String>,
    /// The tool's own default, shown when the option is not set.
    pub default: String,
    pub help: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    pub encoder: EncoderKind,
    pub program: String,
    pub help_args: Vec<String>,
    #[serde(rename = "option")]
    pub options: Vec<Opt>,
}

const SCHEMAS: &[(EncoderKind, &str)] = &[
    (
        EncoderKind::X265,
        include_str!("../../../schemas/x265.toml"),
    ),
    (
        EncoderKind::X264,
        include_str!("../../../schemas/x264.toml"),
    ),
    (
        EncoderKind::SvtAv1,
        include_str!("../../../schemas/svt_av1.toml"),
    ),
    (
        EncoderKind::Rav1e,
        include_str!("../../../schemas/rav1e.toml"),
    ),
    (
        EncoderKind::NvEncC,
        include_str!("../../../schemas/nvencc.toml"),
    ),
    (
        EncoderKind::QsvEncC,
        include_str!("../../../schemas/qsvencc.toml"),
    ),
    (
        EncoderKind::VceEncC,
        include_str!("../../../schemas/vceencc.toml"),
    ),
    (
        EncoderKind::FfmpegVaapi,
        include_str!("../../../schemas/ffmpeg_vaapi.toml"),
    ),
];

pub fn schema(kind: EncoderKind) -> Schema {
    let text = SCHEMAS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, t)| *t)
        .expect("every encoder has a schema");
    toml::from_str(text).expect("built-in schemas parse (tested)")
}

/// The preset key holding this encoder's settings ("x265", "svt_av1"...).
pub fn preset_key(kind: EncoderKind) -> &'static str {
    match kind {
        EncoderKind::X265 => "x265",
        EncoderKind::X264 => "x264",
        EncoderKind::SvtAv1 => "svt_av1",
        EncoderKind::Rav1e => "rav1e",
        EncoderKind::NvEncC => "nvencc",
        EncoderKind::QsvEncC => "qsvencc",
        EncoderKind::VceEncC => "vceencc",
        EncoderKind::FfmpegVaapi => "ffmpeg_vaapi",
    }
}

/// A token that starts a new option: "-x" or "--x" with a letter after
/// the dashes ("-1:-1" is a value).
fn is_flag(s: &str) -> bool {
    let t = s.trim_start_matches('-');
    s.starts_with('-')
        && t.len() < s.len()
        && t.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
}

/// Split an argument list into known options (flag -> value; "on"/"off"
/// for toggles and flags) and everything else, in order.
pub fn parse(schema: &Schema, args: &[String]) -> (BTreeMap<String, String>, Vec<String>) {
    let mut known = BTreeMap::new();
    let mut other = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let next_value = args.get(i + 1).filter(|n| !is_flag(n));
        if let Some(o) = schema.options.iter().find(|o| &o.flag == a) {
            match o.kind {
                OptKind::Toggle | OptKind::Flag => {
                    known.insert(o.flag.clone(), "on".into());
                }
                _ => {
                    if let Some(v) = next_value {
                        known.insert(o.flag.clone(), v.clone());
                        i += 1;
                    }
                }
            }
        } else if let Some(o) = schema.options.iter().find(|o| o.off.as_deref() == Some(a)) {
            known.insert(o.flag.clone(), "off".into());
        } else {
            other.push(a.clone());
            if is_flag(a) {
                if let Some(v) = next_value {
                    other.push(v.clone());
                    i += 1;
                }
            }
        }
        i += 1;
    }
    (known, other)
}

/// Write known options (in schema order) then the other arguments.
pub fn render(schema: &Schema, known: &BTreeMap<String, String>, other: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for o in &schema.options {
        let Some(v) = known.get(&o.flag) else {
            continue;
        };
        match o.kind {
            OptKind::Toggle => match (v.as_str(), &o.off) {
                ("on", _) => out.push(o.flag.clone()),
                ("off", Some(off)) => out.push(off.clone()),
                _ => {}
            },
            OptKind::Flag => {
                if v == "on" {
                    out.push(o.flag.clone());
                }
            }
            _ => {
                out.push(o.flag.clone());
                out.push(v.clone());
            }
        }
    }
    out.extend(other.iter().cloned());
    out
}

/// Check a value for an option. Empty means "not set" and is always fine.
pub fn validate(o: &Opt, value: &str) -> std::result::Result<(), String> {
    let v = value.trim();
    if v.is_empty() {
        return Ok(());
    }
    let range = |n: f64| -> std::result::Result<(), String> {
        if o.min.is_some_and(|m| n < m) || o.max.is_some_and(|m| n > m) {
            return Err(format!(
                "{}: {v} is outside {} to {}",
                o.label,
                o.min.map_or("-".into(), |m| m.to_string()),
                o.max.map_or("-".into(), |m| m.to_string())
            ));
        }
        Ok(())
    };
    match o.kind {
        OptKind::Int => range(
            v.parse::<i64>()
                .map_err(|_| format!("{}: '{v}' is not a whole number", o.label))?
                as f64,
        ),
        OptKind::Float => range(
            v.parse::<f64>()
                .map_err(|_| format!("{}: '{v}' is not a number", o.label))?,
        ),
        OptKind::Choice => {
            if o.choices.iter().any(|c| c == v) {
                Ok(())
            } else {
                Err(format!(
                    "{}: choose one of {}",
                    o.label,
                    o.choices.join(", ")
                ))
            }
        }
        OptKind::Toggle => match v {
            "on" | "off" => Ok(()),
            _ => Err(format!("{}: on or off", o.label)),
        },
        OptKind::Flag => match v {
            "on" | "off" => Ok(()),
            _ => Err(format!("{}: on or off", o.label)),
        },
        OptKind::Text => {
            if v.chars().any(char::is_whitespace) {
                return Err(format!("{}: no spaces", o.label));
            }
            match &o.pattern {
                Some(p) if !simple_match(p, v) => Err(format!(
                    "{}: '{v}' does not look right ({})",
                    o.label, o.help
                )),
                _ => Ok(()),
            }
        }
    }
}

/// The tiny pattern language used by schemas: `^[set]+$`, `^-?[0-6]:-?[0-6]$`
/// style patterns with literal characters, optional `-?` and `[..]`/`[..]+`
/// classes. Enough to catch typos without a regex dependency.
fn simple_match(pattern: &str, text: &str) -> bool {
    enum Tok {
        Lit(char, bool),
        Class(Vec<(char, char)>, bool, bool),
    }
    let p: Vec<char> = pattern
        .trim_start_matches('^')
        .trim_end_matches('$')
        .chars()
        .collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < p.len() {
        let tok = if p[i] == '[' {
            let mut set = Vec::new();
            i += 1;
            while i < p.len() && p[i] != ']' {
                if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
                    set.push((p[i], p[i + 2]));
                    i += 3;
                } else {
                    set.push((p[i], p[i]));
                    i += 1;
                }
            }
            i += 1;
            let plus = p.get(i) == Some(&'+');
            if plus {
                i += 1;
            }
            Tok::Class(set, plus, false)
        } else {
            let c = if p[i] == '\\' {
                i += 1;
                p[i]
            } else {
                p[i]
            };
            i += 1;
            Tok::Lit(c, false)
        };
        let optional = p.get(i) == Some(&'?');
        if optional {
            i += 1;
        }
        toks.push(match tok {
            Tok::Lit(c, _) => Tok::Lit(c, optional),
            Tok::Class(s, plus, _) => Tok::Class(s, plus, optional),
        });
    }
    fn go(toks: &[Tok], t: &[char]) -> bool {
        let Some((first, rest)) = toks.split_first() else {
            return t.is_empty();
        };
        match first {
            Tok::Lit(c, opt) => {
                (t.first() == Some(c) && go(rest, &t[1..])) || (*opt && go(rest, t))
            }
            Tok::Class(set, plus, opt) => {
                let inside = |c: char| set.iter().any(|(a, b)| (*a..=*b).contains(&c));
                if *opt && go(rest, t) {
                    return true;
                }
                let mut n = 0;
                while n < t.len() && inside(t[n]) {
                    n += 1;
                    if go(rest, &t[n..]) {
                        return true;
                    }
                    if !plus {
                        break;
                    }
                }
                false
            }
        }
    }
    let t: Vec<char> = text.chars().collect();
    go(&toks, &t)
}

/// Set one option in an argument list; an empty value removes it.
pub fn set(schema: &Schema, args: &[String], flag: &str, value: &str) -> Result<Vec<String>> {
    let o = schema
        .options
        .iter()
        .find(|o| o.flag == flag)
        .ok_or_else(|| {
            Error::Preset(format!(
                "{flag} is not on the {} option page",
                schema.program
            ))
        })?;
    validate(o, value).map_err(Error::Preset)?;
    let (mut known, other) = parse(schema, args);
    let v = value.trim();
    if v.is_empty() || ((o.kind == OptKind::Flag) && v == "off") {
        known.remove(flag);
    } else {
        known.insert(flag.to_string(), v.to_string());
    }
    Ok(render(schema, &known, &other))
}

/// Replace the "other arguments" part, keeping the known options.
pub fn set_other(schema: &Schema, args: &[String], other: &str) -> Result<Vec<String>> {
    let (known, _) = parse(schema, args);
    let words = shell_words(other)?;
    // Known options typed into the free field are picked up properly.
    let (more, rest) = parse(schema, &words);
    let mut known = known;
    for (k, v) in more {
        let o = schema
            .options
            .iter()
            .find(|o| o.flag == k)
            .expect("parsed from schema");
        validate(o, &v).map_err(Error::Preset)?;
        known.insert(k, v);
    }
    Ok(render(schema, &known, &rest))
}

/// Split like a shell: spaces separate, quotes group.
pub fn shell_words(s: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if quote.is_some() {
        return Err(Error::Preset(
            "unmatched quote in the other arguments".into(),
        ));
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

/// Every option token in a tool's help text.
pub fn help_flags(help: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    // x265 writes "--[no-]sao" for both forms.
    let mut expanded = String::with_capacity(help.len() * 2);
    for word in help.split_whitespace() {
        if let Some(base) = word.strip_prefix("--[no-]") {
            expanded.push_str(&format!(" --{base} --no-{base} "));
        } else {
            expanded.push_str(word);
            expanded.push(' ');
        }
    }
    for word in expanded
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '=' | '[' | '(' | '<' | '|' | '/'))
    {
        let w = word.trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
        if w.len() > 1
            && is_flag(w)
            && w.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            out.insert(w.to_string());
        }
    }
    out
}

/// Schema flags the tool's help does not mention (renamed or removed).
pub fn missing_from_help(schema: &Schema, help: &str) -> Vec<String> {
    let flags = help_flags(help);
    let mut missing = Vec::new();
    for o in &schema.options {
        for f in std::iter::once(&o.flag).chain(o.off.iter()) {
            if !flags.contains(f) {
                missing.push(f.clone());
            }
        }
    }
    missing
}

/// Rows for the option page: flag, label, group, kind, value, default,
/// choices ("|"), min, max, help.
pub fn page_rows(schema: &Schema, args: &[String]) -> (Vec<[String; 10]>, String) {
    let (known, other) = parse(schema, args);
    let rows = schema
        .options
        .iter()
        .map(|o| {
            [
                o.flag.clone(),
                o.label.clone(),
                o.group.clone(),
                format!("{:?}", o.kind).to_lowercase(),
                known.get(&o.flag).cloned().unwrap_or_default(),
                o.default.clone(),
                o.choices.join("|"),
                o.min.map(|m| m.to_string()).unwrap_or_default(),
                o.max.map(|m| m.to_string()).unwrap_or_default(),
                o.help.clone(),
            ]
        })
        .collect();
    let other = other
        .iter()
        .map(|a| {
            if a.contains(' ') {
                format!("'{a}'")
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    (rows, other)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn every_schema_parses_and_is_sane() {
        for kind in EncoderKind::ALL {
            let s = schema(kind);
            assert_eq!(s.encoder, kind);
            let mut seen = BTreeSet::new();
            for o in &s.options {
                assert!(seen.insert(o.flag.clone()), "{kind:?}: {} twice", o.flag);
                assert!(!o.help.is_empty() && !o.label.is_empty());
                if o.kind == OptKind::Choice {
                    assert!(!o.choices.is_empty(), "{}", o.flag);
                }
                if o.kind == OptKind::Toggle {
                    assert!(o.off.is_some(), "{} needs an off flag", o.flag);
                }
                // Anvil sets these itself; they must not be on a page.
                for managed in [
                    "--crf",
                    "--preset",
                    "--colorprim",
                    "--master-display",
                    "--dolby-vision-rpu",
                    "--output-depth",
                    "--qvbr",
                    "--icq",
                    "--cqp",
                    "--quantizer",
                    "--speed",
                ] {
                    assert_ne!(o.flag, managed, "{kind:?}");
                }
            }
        }
    }

    #[test]
    fn parse_set_render_round_trip() {
        let s = schema(EncoderKind::X265);
        // The Quality Default's extra arguments.
        let args = v(&[
            "--aq-mode",
            "3",
            "--no-sao",
            "--deblock",
            "-1:-1",
            "--some-new-flag",
            "5",
            "--bare",
        ]);
        let (known, other) = parse(&s, &args);
        assert_eq!(known["--aq-mode"], "3");
        assert_eq!(known["--sao"], "off");
        assert_eq!(known["--deblock"], "-1:-1");
        assert_eq!(other, v(&["--some-new-flag", "5", "--bare"]));
        // Writing back keeps the meaning (schema order).
        let back = render(&s, &known, &other);
        assert_eq!(parse(&s, &back), (known, other));

        let a = set(&s, &args, "--psy-rdoq", "2.5").unwrap();
        assert!(a.windows(2).any(|w| w == ["--psy-rdoq", "2.5"]));
        let a = set(&s, &a, "--sao", "on").unwrap();
        assert!(a.contains(&"--sao".into()) && !a.contains(&"--no-sao".into()));
        let a = set(&s, &a, "--aq-mode", "").unwrap();
        assert!(!a.contains(&"--aq-mode".into()));
        assert!(set(&s, &a, "--bframes", "40")
            .unwrap_err()
            .to_string()
            .contains("outside 0 to 16"));
        assert!(set(&s, &a, "--me", "warp").is_err());
        assert!(set(&s, &a, "--deblock", "soft").is_err());
        assert!(set(&s, &a, "--crf", "18").is_err(), "not on the page");

        let a = set_other(&s, &a, "--bframes 6 --zones '0,100,q=20'").unwrap();
        assert!(a.windows(2).any(|w| w == ["--bframes", "6"]));
        assert!(a.ends_with(&v(&["--zones", "0,100,q=20"])));
        // The free field replaces the other arguments.
        assert_eq!(page_rows(&s, &a).1, "--zones 0,100,q=20");
    }

    #[test]
    fn flags_and_patterns() {
        let s = schema(EncoderKind::X264);
        let a = set(&s, &[], "--no-fast-pskip", "on").unwrap();
        assert_eq!(a, v(&["--no-fast-pskip"]));
        assert!(set(&s, &a, "--no-fast-pskip", "off").unwrap().is_empty());
        assert!(simple_match("^-?[0-6]:-?[0-6]$", "-1:-1"));
        assert!(simple_match("^-?[0-6]:-?[0-6]$", "2:0"));
        assert!(!simple_match("^-?[0-6]:-?[0-6]$", "-1"));
        assert!(simple_match("^[0-9.]+:[0-9.]+$", "1.0:0.15"));
        assert!(simple_match("^[0-9+*,-]+$", "8,8"));
        assert!(!is_flag("-1:-1"));
        assert!(is_flag("-bf") && is_flag("--sao"));
    }

    /// The `--help` diff: every schema option must still exist in the help
    /// text of the tool version the schema was written against. New tool
    /// versions get a new file in schemas/help/ and this test says which
    /// options changed.
    #[test]
    fn schemas_match_saved_help_texts() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/help");
        let mut checked = 0;
        for kind in EncoderKind::ALL {
            let s = schema(kind);
            let prefix = match kind {
                EncoderKind::X265 => "x265-",
                EncoderKind::X264 => "x264-",
                EncoderKind::SvtAv1 => "svt-av1",
                EncoderKind::Rav1e => "rav1e-",
                EncoderKind::FfmpegVaapi => "ffmpeg-",
                // No saved help yet (no hardware here): checked at run
                // time with `anvil options --check`.
                _ => continue,
            };
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with(prefix) {
                    continue;
                }
                let help = std::fs::read_to_string(entry.path()).unwrap();
                let missing = missing_from_help(&s, &help);
                assert!(
                    missing.is_empty(),
                    "{name}: schema options not in help: {missing:?}"
                );
                checked += 1;
            }
        }
        assert!(checked >= 5, "help texts found: {checked}");
    }
}
