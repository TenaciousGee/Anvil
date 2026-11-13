// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Layered TOML presets.
//!
//! Resolution order: built-in base, then any `inherits` chain, then the
//! user's file, then per-job overrides. Each layer stores only the keys it
//! changes, so improvements to built-ins flow through to user presets.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use toml::Value;

use crate::audio::AudioRules;
use crate::encoder::{
    normalise_codec, EncoderKind, HwSettings, NvEncSettings, Rav1eSettings, SvtAv1Settings,
    VaapiSettings, X264Settings, X265Settings,
};
use crate::scriptgen::FilterProfile;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub video: VideoSection,
    pub filters: FilterProfile,
    pub audio: AudioRules,
    pub subtitles: SubtitleRules,
    pub output: OutputSection,
    pub hdr: HdrSection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoSection {
    pub encoder: EncoderKind,
    pub x265: X265Settings,
    pub svt_av1: SvtAv1Settings,
    pub nvencc: NvEncSettings,
    #[serde(default = "HwSettings::qsv_default")]
    pub qsvencc: HwSettings,
    #[serde(default = "HwSettings::vce_default")]
    pub vceencc: HwSettings,
    #[serde(default)]
    pub ffmpeg_vaapi: VaapiSettings,
    #[serde(default)]
    pub x264: X264Settings,
    #[serde(default)]
    pub rav1e: Rav1eSettings,
}

impl VideoSection {
    /// The output codec: "hevc", "av1" or "h264".
    pub fn codec(&self) -> String {
        match self.encoder {
            EncoderKind::X265 => "hevc".into(),
            EncoderKind::X264 => "h264".into(),
            EncoderKind::SvtAv1 | EncoderKind::Rav1e => "av1".into(),
            EncoderKind::NvEncC => normalise_codec(&self.nvencc.codec),
            EncoderKind::QsvEncC => normalise_codec(&self.qsvencc.codec),
            EncoderKind::VceEncC => normalise_codec(&self.vceencc.codec),
            EncoderKind::FfmpegVaapi => normalise_codec(&self.ffmpeg_vaapi.codec),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtitleRules {
    /// Languages to keep; empty keeps every subtitle track.
    #[serde(default)]
    pub languages: Vec<String>,
    /// Always keep forced tracks, whatever their language.
    #[serde(default = "yes")]
    pub keep_forced: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputSection {
    pub container: String,
    pub name_template: String,
    pub auto_crop: bool,
    pub keep_chapters: bool,
    /// Copy the original video untouched; only audio, subtitles and
    /// chapters are processed.
    pub video_passthrough: bool,
    /// Keep the finished file at or under this share of the source's size
    /// (audio included). The CRF is raised only as far as needed.
    #[serde(default)]
    pub max_size_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HdrSection {
    pub keep_dolby_vision: bool,
    pub keep_hdr10_plus: bool,
}

fn yes() -> bool {
    true
}

/// Built-in presets, keyed by id (file stem).
const BUILTINS: &[(&str, &str)] = &[
    (
        "quality-default",
        include_str!("../../../presets/quality-default.toml"),
    ),
    (
        "av1-archive",
        include_str!("../../../presets/av1-archive.toml"),
    ),
    ("fast-qsv", include_str!("../../../presets/fast-qsv.toml")),
    ("fast-amd", include_str!("../../../presets/fast-amd.toml")),
    (
        "fast-vaapi",
        include_str!("../../../presets/fast-vaapi.toml"),
    ),
    (
        "compat-x264",
        include_str!("../../../presets/compat-x264.toml"),
    ),
    ("av1-rav1e", include_str!("../../../presets/av1-rav1e.toml")),
    (
        "fast-nvenc",
        include_str!("../../../presets/fast-nvenc.toml"),
    ),
];

pub const DEFAULT_PRESET: &str = "quality-default";

/// All known preset layers, built-in and user, as raw TOML tables.
#[derive(Debug, Clone)]
pub struct PresetStore {
    layers: BTreeMap<String, (Value, Origin)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    BuiltIn,
    User,
}

impl PresetStore {
    pub fn builtin() -> Result<Self> {
        let mut layers = BTreeMap::new();
        for (id, text) in BUILTINS {
            let v: Value = toml::from_str(text)?;
            layers.insert(id.to_string(), (v, Origin::BuiltIn));
        }
        Ok(PresetStore { layers })
    }

    /// Add user presets from a directory of `*.toml` files. A user file with
    /// the same id as a built-in is layered on top of it rather than
    /// replacing it.
    pub fn load_user_dir(&mut self, dir: &Path) -> Result<()> {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(Error::Io {
                    path: dir.into(),
                    source,
                })
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let Some(id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            let text = std::fs::read_to_string(&path).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            self.add_user(&id, &text)?;
        }
        Ok(())
    }

    pub fn add_user(&mut self, id: &str, text: &str) -> Result<()> {
        let mut v: Value = toml::from_str(text)?;
        if let Some((base, Origin::BuiltIn)) = self.layers.get(id) {
            let mut merged = base.clone();
            deep_merge(&mut merged, v);
            v = merged;
        }
        self.layers.insert(id.to_string(), (v, Origin::User));
        Ok(())
    }

    pub fn ids(&self) -> impl Iterator<Item = (&str, Origin)> {
        self.layers.iter().map(|(k, (_, o))| (k.as_str(), *o))
    }

    /// Resolve a preset id, following `inherits`, then apply an optional
    /// per-job override table.
    pub fn resolve(&self, id: &str, job_override: Option<&Value>) -> Result<Preset> {
        let mut merged = self.resolve_value(id, 0)?;
        if let Some(o) = job_override {
            deep_merge(&mut merged, o.clone());
        }
        if let Value::Table(t) = &mut merged {
            t.remove("inherits");
        }
        merged
            .try_into()
            .map_err(|e: toml::de::Error| Error::Preset(format!("{id}: {e}")))
    }

    fn resolve_value(&self, id: &str, depth: usize) -> Result<Value> {
        if depth > 8 {
            return Err(Error::Preset(format!(
                "inheritance chain too deep at '{id}'"
            )));
        }
        let (layer, _) = self
            .layers
            .get(id)
            .ok_or_else(|| Error::UnknownPreset(id.into()))?;
        match layer.get("inherits").and_then(Value::as_str) {
            Some(parent) => {
                let mut base = self.resolve_value(parent, depth + 1)?;
                deep_merge(&mut base, layer.clone());
                Ok(base)
            }
            None => Ok(layer.clone()),
        }
    }
}

/// A preset rule: when the source matches every condition given, the `set`
/// table is merged into the preset and the filters are added.
///
/// ```toml
/// [[rules]]
/// name = "UHD HDR: slower x265"
/// when = { resolution = "uhd", hdr = true }
/// set = { video = { x265 = { preset = "slow" } } }
/// add_filters = ["grain"]
/// ```
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    #[serde(default)]
    pub when: Condition,
    #[serde(default)]
    pub set: toml::Table,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_filters: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Condition {
    /// "sd", "hd" or "uhd" (from the source's frame size).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    /// Any HDR (HDR10, HLG, Dolby Vision).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdr: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dolby_vision: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interlaced: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longer_than_minutes: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shorter_than_minutes: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub larger_than_gb: Option<f64>,
}

impl Condition {
    pub fn matches(&self, s: &crate::model::SourceInfo) -> bool {
        use crate::model::{DynamicRange, ResolutionClass};
        let v = &s.video;
        let class = match ResolutionClass::from_dimensions(v.width, v.height) {
            ResolutionClass::Sd => "sd",
            ResolutionClass::Hd => "hd",
            ResolutionClass::Uhd => "uhd",
        };
        let minutes = s.duration_secs.map(|d| d / 60.0);
        let gb = s.size_bytes.map(|b| b as f64 / 1e9);
        self.resolution
            .as_deref()
            .is_none_or(|r| r.eq_ignore_ascii_case(class))
            && self
                .hdr
                .is_none_or(|h| h == (v.dynamic_range != DynamicRange::Sdr))
            && self
                .dolby_vision
                .is_none_or(|d| d == v.dolby_vision.is_some())
            && self.interlaced.is_none_or(|i| i == v.interlaced)
            && self
                .longer_than_minutes
                .is_none_or(|m| minutes.is_some_and(|x| x > m))
            && self
                .shorter_than_minutes
                .is_none_or(|m| minutes.is_some_and(|x| x < m))
            && self
                .larger_than_gb
                .is_none_or(|g| gb.is_some_and(|x| x > g))
    }

    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(r) = &self.resolution {
            parts.push(r.to_uppercase());
        }
        for (v, yes, no) in [
            (self.hdr, "HDR", "SDR"),
            (self.dolby_vision, "Dolby Vision", "no Dolby Vision"),
            (self.interlaced, "interlaced", "progressive"),
        ] {
            if let Some(b) = v {
                parts.push(if b { yes } else { no }.into());
            }
        }
        if let Some(m) = self.longer_than_minutes {
            parts.push(format!("over {m} min"));
        }
        if let Some(m) = self.shorter_than_minutes {
            parts.push(format!("under {m} min"));
        }
        if let Some(g) = self.larger_than_gb {
            parts.push(format!("over {g} GB"));
        }
        if parts.is_empty() {
            "every source".into()
        } else {
            parts.join(", ")
        }
    }
}

/// A preset resolved for one source, with the rules that applied.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub preset: Preset,
    pub applied: Vec<String>,
}

impl PresetStore {
    /// The preset's rules (after inheritance; a layer's list replaces its
    /// parent's).
    pub fn rules(&self, id: &str) -> Result<Vec<Rule>> {
        let v = self.resolve_value(id, 0)?;
        rules_of(&v)
    }

    /// Resolve a preset for a source: its rules are applied before any
    /// per-job override.
    pub fn resolve_for(
        &self,
        id: &str,
        source: &crate::model::SourceInfo,
        job_override: Option<&Value>,
    ) -> Result<Resolved> {
        let mut merged = self.resolve_value(id, 0)?;
        let rules = rules_of(&merged)?;
        let mut applied = Vec::new();
        let mut filters = Vec::new();
        for r in rules.iter().filter(|r| r.when.matches(source)) {
            deep_merge(&mut merged, Value::Table(r.set.clone()));
            filters.extend(r.add_filters.iter().cloned());
            applied.push(r.name.clone());
        }
        if let Some(o) = job_override {
            deep_merge(&mut merged, o.clone());
        }
        if let Value::Table(t) = &mut merged {
            t.remove("inherits");
            t.remove("rules");
        }
        let mut preset: Preset = merged
            .try_into()
            .map_err(|e: toml::de::Error| Error::Preset(format!("{id}: {e}")))?;
        let order = if source.video.top_field_first {
            crate::scriptgen::FieldOrder::TopFieldFirst
        } else {
            crate::scriptgen::FieldOrder::BottomFieldFirst
        };
        for f in filters {
            let block = crate::scriptgen::new_block(&f, order)
                .ok_or_else(|| Error::Preset(format!("{id}: a rule adds unknown filter '{f}'")))?;
            if !preset.filters.blocks.iter().any(|b| b.id() == block.id()) {
                preset.filters.blocks.push(block);
            }
        }
        Ok(Resolved { preset, applied })
    }

    /// The full TOML of a preset (after inheritance), rules included: the
    /// starting point for editing.
    pub fn full_value(&self, id: &str) -> Result<Value> {
        let mut v = self.resolve_value(id, 0)?;
        if let Value::Table(t) = &mut v {
            t.remove("inherits");
        }
        Ok(v)
    }

    /// Where a preset's layers start: a user preset's parent, or itself.
    pub fn parent_of(&self, id: &str) -> Option<String> {
        let (layer, origin) = self.layers.get(id)?;
        match origin {
            Origin::BuiltIn => Some(id.to_string()),
            Origin::User => layer
                .get("inherits")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    BUILTINS
                        .iter()
                        .any(|(b, _)| *b == id)
                        .then(|| id.to_string())
                }),
        }
    }

    pub fn origin(&self, id: &str) -> Option<Origin> {
        self.layers.get(id).map(|(_, o)| *o)
    }
}

fn rules_of(v: &Value) -> Result<Vec<Rule>> {
    match v.get("rules") {
        None => Ok(Vec::new()),
        Some(r) => r
            .clone()
            .try_into()
            .map_err(|e: toml::de::Error| Error::Preset(format!("rules: {e}"))),
    }
}

/// The keys of `edited` that differ from `base`: what a user preset stores.
pub fn overlay_diff(base: &Value, edited: &Value) -> Option<Value> {
    match (base, edited) {
        (Value::Table(b), Value::Table(e)) => {
            let mut out = toml::Table::new();
            for (k, v) in e {
                match b.get(k) {
                    Some(bv) => {
                        if let Some(d) = overlay_diff(bv, v) {
                            out.insert(k.clone(), d);
                        }
                    }
                    None => {
                        out.insert(k.clone(), v.clone());
                    }
                }
            }
            (!out.is_empty()).then_some(Value::Table(out))
        }
        (b, e) if b == e => None,
        (_, e) => Some(e.clone()),
    }
}

/// Read a dotted path ("video.x265.crf.uhd") from a TOML value.
pub fn get_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |cur, key| cur.get(key))
}

/// Set a dotted path from text, keeping the type already there (number,
/// whole number, true/false, list of words, text). Missing optional keys
/// take a number when the text is one.
pub fn set_path(v: &mut Value, path: &str, text: &str) -> std::result::Result<(), String> {
    let keys: Vec<&str> = path.split('.').collect();
    let (last, parents) = keys.split_last().ok_or("empty path")?;
    let mut cur = v;
    for k in parents {
        let t = cur
            .as_table_mut()
            .ok_or_else(|| format!("{path}: not a table"))?;
        cur = t
            .entry(k.to_string())
            .or_insert_with(|| Value::Table(toml::Table::new()));
    }
    let table = cur
        .as_table_mut()
        .ok_or_else(|| format!("{path}: not a table"))?;
    let t = text.trim();
    let new = match table.get(*last) {
        Some(Value::Float(_)) => {
            Value::Float(t.parse().map_err(|_| format!("'{t}' is not a number"))?)
        }
        Some(Value::Integer(_)) => match (t.parse::<i64>(), t.parse::<f64>()) {
            (Ok(i), _) => Value::Integer(i),
            (_, Ok(f)) => Value::Float(f),
            _ => return Err(format!("'{t}' is not a number")),
        },
        Some(Value::Boolean(_)) => Value::Boolean(t == "true" || t == "1" || t == "yes"),
        Some(Value::Array(_)) => Value::Array(
            t.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| Value::String(s.to_string()))
                .collect(),
        ),
        Some(_) => Value::String(text.to_string()),
        None => match t.parse::<f64>() {
            Ok(n) => Value::Float(n),
            Err(_) => Value::String(text.to_string()),
        },
    };
    table.insert(last.to_string(), new);
    Ok(())
}

/// Check an edited preset still makes a valid preset.
pub fn validate(v: &Value) -> Result<Preset> {
    let mut v = v.clone();
    if let Value::Table(t) = &mut v {
        t.remove("inherits");
        t.remove("rules");
    }
    v.try_into()
        .map_err(|e: toml::de::Error| Error::Preset(e.to_string()))
}

/// Save an edited preset as a user preset: `inherits` its parent and stores
/// only what differs, rules included.
pub fn user_preset_toml(store: &PresetStore, parent: &str, edited: &Value) -> Result<String> {
    validate(edited)?;
    let base = store.full_value(parent)?;
    let mut out = toml::Table::new();
    out.insert("inherits".into(), Value::String(parent.into()));
    if let Some(Value::Table(d)) = overlay_diff(&base, edited) {
        for (k, v) in d {
            out.insert(k, v);
        }
    }
    // Keep the name even when it matches (a user preset needs its own).
    if let Some(n) = edited.get("name") {
        out.insert("name".into(), n.clone());
    }
    Ok(format!(
        "# Anvil user preset: stores only what differs from \"{parent}\".\n{}",
        toml::to_string_pretty(&out).map_err(|e| Error::Preset(e.to_string()))?
    ))
}

/// A rule's `set` table as "a.b = value; c = value".
pub fn flatten(t: &toml::Table) -> String {
    fn walk(prefix: &str, t: &toml::Table, out: &mut Vec<String>) {
        for (k, v) in t {
            let path = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{prefix}.{k}")
            };
            match v {
                Value::Table(inner) => walk(&path, inner, out),
                v => out.push(format!("{path} = {v}")),
            }
        }
    }
    let mut out = Vec::new();
    walk("", t, &mut out);
    out.join("; ")
}

/// Parse "a.b = value; c = value" back into a table.
pub fn unflatten(text: &str) -> std::result::Result<toml::Table, String> {
    let mut root = Value::Table(toml::Table::new());
    for part in text
        .split([';', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let (path, raw) = part
            .split_once('=')
            .ok_or_else(|| format!("'{part}' needs the form setting = value"))?;
        let path = path.trim();
        let raw = raw.trim();
        let value = format!("v = {raw}")
            .parse::<toml::Table>()
            .ok()
            .and_then(|mut t| t.remove("v"))
            // Bare words are text: preset = slow
            .unwrap_or_else(|| Value::String(raw.trim_matches('"').to_string()));
        let keys: Vec<&str> = path.split('.').map(str::trim).collect();
        let (last, parents) = keys.split_last().ok_or("empty setting name")?;
        let mut cur = &mut root;
        for k in parents {
            cur = cur
                .as_table_mut()
                .ok_or_else(|| format!("{path}: not a table"))?
                .entry(k.to_string())
                .or_insert_with(|| Value::Table(toml::Table::new()));
        }
        cur.as_table_mut()
            .ok_or_else(|| format!("{path}: not a table"))?
            .insert(last.to_string(), value);
    }
    match root {
        Value::Table(t) => Ok(t),
        _ => unreachable!(),
    }
}

/// Recursively merge `overlay` into `base`. Tables merge key by key; any
/// other value (including arrays) replaces the base value.
pub fn deep_merge(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Table(b), Value::Table(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(existing) => deep_merge(existing, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_resolve() {
        let store = PresetStore::builtin().unwrap();
        let q = store.resolve("quality-default", None).unwrap();
        assert_eq!(q.video.encoder, EncoderKind::X265);
        assert_eq!(q.video.x265.crf.uhd, 18.0);
        assert_eq!(q.audio.ac3_bitrate_multichannel, 640);

        let av1 = store.resolve("av1-archive", None).unwrap();
        assert_eq!(av1.video.encoder, EncoderKind::SvtAv1);
        // Inherited untouched from the base.
        assert_eq!(av1.audio, q.audio);

        assert_eq!(
            store.resolve("fast-nvenc", None).unwrap().video.encoder,
            EncoderKind::NvEncC
        );
    }

    #[test]
    fn user_layer_and_job_override() {
        let mut store = PresetStore::builtin().unwrap();
        store
            .add_user("my-films", "inherits = \"quality-default\"\nname = \"My films\"\n[video.x265]\npreset = \"slower\"\n")
            .unwrap();
        let mine = store.resolve("my-films", None).unwrap();
        assert_eq!(mine.video.x265.preset, "slower");
        assert_eq!(mine.video.x265.crf.hd, 18.0, "unchanged keys inherit");

        let job: Value = toml::from_str("[video.x265.crf]\nhd = 16.0").unwrap();
        let tuned = store.resolve("my-films", Some(&job)).unwrap();
        assert_eq!(tuned.video.x265.crf.hd, 16.0);
        assert_eq!(tuned.video.x265.crf.sd, 17.0);
    }

    #[test]
    fn unknown_and_cyclic() {
        let mut store = PresetStore::builtin().unwrap();
        assert!(matches!(
            store.resolve("nope", None),
            Err(Error::UnknownPreset(_))
        ));
        store.add_user("a", "inherits = \"b\"").unwrap();
        store.add_user("b", "inherits = \"a\"").unwrap();
        assert!(store.resolve("a", None).is_err());
    }

    fn source(w: u32, h: u32, hdr: bool, minutes: f64) -> crate::model::SourceInfo {
        let mut s = crate::probe::parse_ffprobe_json(include_str!(
            "../tests/fixtures/uhd_dovi_p7_truehd.json"
        ))
        .unwrap();
        s.video.width = w;
        s.video.height = h;
        if !hdr {
            s.video.dynamic_range = crate::model::DynamicRange::Sdr;
            s.video.dolby_vision = None;
        }
        s.duration_secs = Some(minutes * 60.0);
        s
    }

    #[test]
    fn rules_apply_per_source() {
        let mut store = PresetStore::builtin().unwrap();
        store
            .add_user(
                "films",
                r#"inherits = "quality-default"
name = "Films"

[[rules]]
name = "UHD HDR: slower"
when = { resolution = "uhd", hdr = true }
set = { video = { x265 = { preset = "slow" } } }

[[rules]]
name = "Long SD: grain"
when = { resolution = "sd", longer_than_minutes = 60 }
set = { video = { x265 = { crf = { sd = 19.0 } } } }
add_filters = ["grain"]
"#,
            )
            .unwrap();
        let uhd = store
            .resolve_for("films", &source(3840, 2160, true, 120.0), None)
            .unwrap();
        assert_eq!(uhd.applied, vec!["UHD HDR: slower"]);
        assert_eq!(uhd.preset.video.x265.preset, "slow");
        assert!(uhd.preset.filters.blocks.is_empty());

        let sdr_uhd = store
            .resolve_for("films", &source(3840, 2160, false, 120.0), None)
            .unwrap();
        assert!(sdr_uhd.applied.is_empty());
        assert_eq!(sdr_uhd.preset.video.x265.preset, "medium");

        let dvd = store
            .resolve_for("films", &source(720, 480, false, 95.0), None)
            .unwrap();
        assert_eq!(dvd.applied, vec!["Long SD: grain"]);
        assert_eq!(dvd.preset.video.x265.crf.sd, 19.0);
        assert_eq!(dvd.preset.filters.blocks.len(), 1);
        let short = store
            .resolve_for("films", &source(720, 480, false, 22.0), None)
            .unwrap();
        assert!(short.applied.is_empty());

        // A job override still wins over a rule.
        let job: Value = toml::from_str("[video.x265]\npreset = \"veryslow\"").unwrap();
        let r = store
            .resolve_for("films", &source(3840, 2160, true, 120.0), Some(&job))
            .unwrap();
        assert_eq!(r.preset.video.x265.preset, "veryslow");

        assert_eq!(
            store.rules("films").unwrap()[1].when.describe(),
            "SD, over 60 min"
        );
        assert_eq!(Condition::default().describe(), "every source");
    }

    #[test]
    fn edit_and_save_as_overlay() {
        let store = PresetStore::builtin().unwrap();
        let mut v = store.full_value("quality-default").unwrap();
        set_path(&mut v, "name", "My UHD").unwrap();
        set_path(&mut v, "video.x265.crf.uhd", "20").unwrap();
        set_path(&mut v, "output.auto_crop", "false").unwrap();
        set_path(&mut v, "subtitles.languages", "eng, fre").unwrap();
        assert!(set_path(&mut v, "video.x265.crf.hd", "soft").is_err());
        assert_eq!(
            get_path(&v, "video.x265.crf.uhd"),
            Some(&Value::Float(20.0))
        );
        let p = validate(&v).unwrap();
        assert_eq!(p.subtitles.languages, vec!["eng", "fre"]);

        let text = user_preset_toml(&store, "quality-default", &v).unwrap();
        assert!(text.contains("inherits = \"quality-default\""));
        assert!(text.contains("uhd = 20.0"));
        assert!(
            !text.contains("hd = 18.0"),
            "unchanged keys stay out: {text}"
        );
        let mut store2 = PresetStore::builtin().unwrap();
        store2.add_user("my-uhd", &text).unwrap();
        let back = store2.resolve("my-uhd", None).unwrap();
        assert_eq!(back.name, "My UHD");
        assert_eq!(back.video.x265.crf.uhd, 20.0);
        assert_eq!(back.video.x265.crf.hd, 18.0);
        assert!(!back.output.auto_crop);

        // Breaking the type is caught before saving.
        let mut bad = store.full_value("quality-default").unwrap();
        set_path(&mut bad, "video.encoder", "magic").unwrap();
        assert!(validate(&bad).is_err());
    }

    #[test]
    fn flatten_round_trip() {
        let t = unflatten(
            "video.x265.preset = slow; video.x265.crf.uhd = 20.0; output.auto_crop = false",
        )
        .unwrap();
        assert_eq!(t["video"]["x265"]["preset"].as_str(), Some("slow"));
        let text = flatten(&t);
        assert_eq!(unflatten(&text).unwrap(), t);
        assert!(text.contains("video.x265.crf.uhd = 20.0"));
        assert!(unflatten("nonsense").is_err());
    }
}
