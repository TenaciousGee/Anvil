// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! The preset editor: a draft copy of a preset (after inheritance) that the
//! Presets page edits field by field, plus its conditional rules. Saving
//! writes a user preset to `~/.config/anvil/presets/<id>.toml` that
//! `inherits` the preset it started from and stores only what differs.

use std::pin::Pin;

use anvil_core::encoder::EncoderKind;
use anvil_core::preset::{
    self, flatten, get_path, set_path, unflatten, user_preset_toml, Origin, Rule,
};
use anvil_core::schema;
use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use toml::Value;

use crate::bridge::{store, user_preset_dir};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// The preset being edited (its id).
        #[qproperty(QString, base)]
        /// Editable settings, one per line: path, label, kind
        /// (text/number/bool/choice/list), value, choices ("id=label|...").
        #[qproperty(QString, fields)]
        /// Rules, one per line: name, resolution, hdr, dv, interlaced,
        /// longer, shorter, larger, settings, filters, summary.
        #[qproperty(QString, rules)]
        #[qproperty(bool, dirty)]
        #[qproperty(bool, active)]
        /// The last problem (empty when all is well).
        #[qproperty(QString, message)]
        /// What saving would write.
        #[qproperty(QString, preview)]
        /// A suggested id for "save as".
        #[qproperty(QString, suggested)]
        /// The encoder whose option page is shown (preset key, e.g. x265).
        #[qproperty(QString, optencoder)]
        /// Option page rows: flag, label, group, kind, value, default,
        /// choices ("|"), min, max, help.
        #[qproperty(QString, options)]
        /// Arguments not on the option page, as typed.
        #[qproperty(QString, other)]
        type PresetEditor = super::PresetEditorRust;

        /// Start editing a preset (built-in or user).
        #[qinvokable]
        fn start(self: Pin<&mut PresetEditor>, id: &QString);
        /// Stop editing and drop the draft.
        #[qinvokable]
        fn discard(self: Pin<&mut PresetEditor>);
        /// Set one setting by its path ("video.x265.crf.uhd").
        #[qinvokable]
        fn set(self: Pin<&mut PresetEditor>, path: &QString, value: &QString);
        #[qinvokable]
        fn ruleadd(self: Pin<&mut PresetEditor>);
        #[qinvokable]
        fn ruleremove(self: Pin<&mut PresetEditor>, index: i32);
        #[qinvokable]
        fn rulemove(self: Pin<&mut PresetEditor>, from: i32, to: i32);
        /// Change one part of a rule: name, resolution, hdr, dv, interlaced,
        /// longer, shorter, larger, settings or filters.
        #[qinvokable]
        fn ruleset(self: Pin<&mut PresetEditor>, index: i32, key: &QString, value: &QString);
        /// Save as a user preset; returns the id saved (empty on failure).
        #[qinvokable]
        fn save(self: Pin<&mut PresetEditor>, id: &QString) -> QString;
        /// Show another encoder's option page.
        #[qinvokable]
        fn optview(self: Pin<&mut PresetEditor>, encoder: &QString);
        /// Set an option on the shown page; empty value resets it.
        #[qinvokable]
        fn optset(self: Pin<&mut PresetEditor>, flag: &QString, value: &QString);
        /// Replace the other arguments.
        #[qinvokable]
        fn optother(self: Pin<&mut PresetEditor>, text: &QString);
        /// Delete a user preset file; returns true when it was removed.
        #[qinvokable]
        fn remove(self: Pin<&mut PresetEditor>, id: &QString) -> bool;
    }
}

#[derive(Default)]
pub struct PresetEditorRust {
    base: QString,
    fields: QString,
    rules: QString,
    dirty: bool,
    active: bool,
    message: QString,
    preview: QString,
    suggested: QString,
    optencoder: QString,
    options: QString,
    other: QString,
    draft: Option<Value>,
    parent: String,
}

fn q(s: &str) -> QString {
    QString::from(s)
}

fn clean(s: &str) -> String {
    s.replace(['\t', '\n'], " ")
}

const ENCODERS: &str = "x265=x265 (CPU, HEVC)|x264=x264 (CPU, AVC 8-bit, SDR only)|svt_av1=SVT-AV1 (CPU, AV1)|rav1e=rav1e (CPU, AV1)|nvencc=NVEncC (NVIDIA GPU)|qsvencc=QSVEncC (Intel GPU)|vceencc=VCEEncC (AMD GPU)|ffmpeg_vaapi=FFmpeg VAAPI (Intel or AMD GPU)";
const X26X_PRESETS: &str = "ultrafast=ultrafast|superfast=superfast|veryfast=veryfast|faster=faster|fast=fast|medium=medium|slow=slow|slower=slower|veryslow=veryslow";
const CODECS: &str = "hevc=HEVC|av1=AV1|h264=H.264";
const NV_PRESETS: &str = "p1=P1 (fastest)|p2=P2|p3=P3|p4=P4 (default)|p5=P5|p6=P6|p7=P7 (best)";
const QSV_PRESETS: &str =
    "best=best|higher=higher|high=high|balanced=balanced|fast=fast|faster=faster|fastest=fastest";
const VCE_PRESETS: &str = "slower=slower|slow=slow|balanced=balanced|fast=fast";

/// (path, label, kind, choices, encoder it belongs to or "")
const FIELDS: &[(&str, &str, &str, &str, &str)] = &[
    ("name", "Name", "text", "", ""),
    ("description", "Description", "text", "", ""),
    ("video.encoder", "Encoder", "choice", ENCODERS, ""),
    (
        "video.x265.preset",
        "x265 speed",
        "choice",
        X26X_PRESETS,
        "x265",
    ),
    ("video.x265.crf.sd", "x265 CRF, SD", "number", "", "x265"),
    ("video.x265.crf.hd", "x265 CRF, HD", "number", "", "x265"),
    ("video.x265.crf.uhd", "x265 CRF, UHD", "number", "", "x265"),
    (
        "video.x264.preset",
        "x264 speed",
        "choice",
        X26X_PRESETS,
        "x264",
    ),
    ("video.x264.crf.sd", "x264 CRF, SD", "number", "", "x264"),
    ("video.x264.crf.hd", "x264 CRF, HD", "number", "", "x264"),
    ("video.x264.crf.uhd", "x264 CRF, UHD", "number", "", "x264"),
    (
        "video.svt_av1.preset",
        "SVT-AV1 preset (0 slow to 13 fast)",
        "number",
        "",
        "svt_av1",
    ),
    (
        "video.svt_av1.crf.hd",
        "SVT-AV1 CRF, HD",
        "number",
        "",
        "svt_av1",
    ),
    (
        "video.svt_av1.crf.uhd",
        "SVT-AV1 CRF, UHD",
        "number",
        "",
        "svt_av1",
    ),
    (
        "video.svt_av1.film_grain",
        "SVT-AV1 film grain (0 to 50)",
        "number",
        "",
        "svt_av1",
    ),
    (
        "video.rav1e.speed",
        "rav1e speed (0 slow to 10 fast)",
        "number",
        "",
        "rav1e",
    ),
    (
        "video.rav1e.quantizer.hd",
        "rav1e quantizer, HD (0 to 255)",
        "number",
        "",
        "rav1e",
    ),
    (
        "video.rav1e.quantizer.uhd",
        "rav1e quantizer, UHD",
        "number",
        "",
        "rav1e",
    ),
    (
        "video.nvencc.codec",
        "NVEncC codec",
        "choice",
        CODECS,
        "nvencc",
    ),
    (
        "video.nvencc.preset",
        "NVEncC preset",
        "choice",
        NV_PRESETS,
        "nvencc",
    ),
    (
        "video.nvencc.quality.hd",
        "NVEncC quality (QVBR), HD",
        "number",
        "",
        "nvencc",
    ),
    (
        "video.nvencc.quality.uhd",
        "NVEncC quality (QVBR), UHD",
        "number",
        "",
        "nvencc",
    ),
    (
        "video.qsvencc.codec",
        "QSVEncC codec",
        "choice",
        CODECS,
        "qsvencc",
    ),
    (
        "video.qsvencc.preset",
        "QSVEncC quality preset",
        "choice",
        QSV_PRESETS,
        "qsvencc",
    ),
    (
        "video.qsvencc.quality.hd",
        "QSVEncC ICQ, HD",
        "number",
        "",
        "qsvencc",
    ),
    (
        "video.qsvencc.quality.uhd",
        "QSVEncC ICQ, UHD",
        "number",
        "",
        "qsvencc",
    ),
    (
        "video.vceencc.codec",
        "VCEEncC codec",
        "choice",
        CODECS,
        "vceencc",
    ),
    (
        "video.vceencc.preset",
        "VCEEncC preset",
        "choice",
        VCE_PRESETS,
        "vceencc",
    ),
    (
        "video.vceencc.quality.hd",
        "VCEEncC QP, HD",
        "number",
        "",
        "vceencc",
    ),
    (
        "video.vceencc.quality.uhd",
        "VCEEncC QP, UHD",
        "number",
        "",
        "vceencc",
    ),
    (
        "video.ffmpeg_vaapi.codec",
        "VAAPI codec",
        "choice",
        CODECS,
        "ffmpeg_vaapi",
    ),
    (
        "video.ffmpeg_vaapi.qp.hd",
        "VAAPI QP, HD",
        "number",
        "",
        "ffmpeg_vaapi",
    ),
    (
        "video.ffmpeg_vaapi.qp.uhd",
        "VAAPI QP, UHD",
        "number",
        "",
        "ffmpeg_vaapi",
    ),
    (
        "video.ffmpeg_vaapi.device",
        "VAAPI device",
        "text",
        "",
        "ffmpeg_vaapi",
    ),
    (
        "output.max_size_percent",
        "Size target, % of source (0 = off)",
        "number",
        "",
        "",
    ),
    (
        "output.auto_crop",
        "Crop black bars automatically",
        "bool",
        "",
        "",
    ),
    ("output.keep_chapters", "Keep chapters", "bool", "", ""),
    (
        "audio.languages",
        "Audio languages (comma separated)",
        "list",
        "",
        "",
    ),
    (
        "audio.ac3_bitrate_multichannel",
        "AC-3 bitrate, surround (kbps)",
        "number",
        "",
        "",
    ),
    (
        "audio.ac3_bitrate_stereo",
        "AC-3 bitrate, stereo (kbps)",
        "number",
        "",
        "",
    ),
    (
        "subtitles.languages",
        "Subtitle languages (empty keeps all)",
        "list",
        "",
        "",
    ),
    (
        "subtitles.keep_forced",
        "Keep forced subtitles",
        "bool",
        "",
        "",
    ),
    ("hdr.keep_dolby_vision", "Keep Dolby Vision", "bool", "", ""),
    ("hdr.keep_hdr10_plus", "Keep HDR10+", "bool", "", ""),
];

fn show(v: Option<&Value>) -> String {
    match v {
        None => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| x.to_string())
            })
            .collect::<Vec<_>>()
            .join(", "),
        Some(Value::Float(f)) if f.fract() == 0.0 => format!("{f:.0}"),
        Some(v) => v.to_string(),
    }
}

fn rules_of(draft: &Value) -> Vec<Rule> {
    draft
        .get("rules")
        .cloned()
        .and_then(|r| r.try_into().ok())
        .unwrap_or_default()
}

fn yes_no(b: Option<bool>) -> &'static str {
    match b {
        None => "any",
        Some(true) => "yes",
        Some(false) => "no",
    }
}

fn num(n: Option<f64>) -> String {
    n.map(|x| x.to_string()).unwrap_or_default()
}

/// Turn a name into a file-safe id.
pub fn slug(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    s.split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

impl qobject::PresetEditor {
    pub fn start(mut self: Pin<&mut Self>, id: &QString) {
        let id = id.to_string();
        let result = store().and_then(|s| {
            let v = s.full_value(&id)?;
            let parent = match s.origin(&id) {
                Some(Origin::User) => s
                    .parent_of(&id)
                    .unwrap_or_else(|| preset::DEFAULT_PRESET.into()),
                _ => id.clone(),
            };
            Ok((v, parent, s.origin(&id)))
        });
        match result {
            Ok((v, parent, origin)) => {
                let suggested = if origin == Some(Origin::User) {
                    id.clone()
                } else {
                    format!("my-{id}")
                };
                {
                    let mut r = self.as_mut().rust_mut();
                    r.draft = Some(v);
                    r.parent = parent;
                }
                self.as_mut().set_suggested(q(&suggested));
                self.as_mut().set_optencoder(QString::default());
                self.as_mut().set_base(q(&id));
                self.as_mut().set_dirty(false);
                self.as_mut().set_message(QString::default());
                self.as_mut().set_active(true);
                self.publish();
            }
            Err(e) => self.set_message(q(&e.to_string())),
        }
    }

    pub fn discard(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().draft = None;
        self.as_mut().set_active(false);
        self.as_mut().set_dirty(false);
        self.set_message(QString::default());
    }

    pub fn set(mut self: Pin<&mut Self>, path: &QString, value: &QString) {
        let (path, value) = (path.to_string(), value.to_string());
        if path == "video.encoder" {
            // The option page follows the chosen encoder.
            self.as_mut().set_optencoder(QString::default());
        }
        self.edit(|d| {
            set_path(d, &path, &value)?;
            preset::validate(d).map(|_| ()).map_err(|e| e.to_string())
        });
    }

    pub fn ruleadd(self: Pin<&mut Self>) {
        self.edit(|d| {
            let mut rules = rules_of(d);
            rules.push(Rule {
                name: format!("Rule {}", rules.len() + 1),
                when: preset::Condition {
                    resolution: Some("uhd".into()),
                    ..Default::default()
                },
                set: unflatten("video.x265.preset = \"slow\"")?,
                add_filters: Vec::new(),
            });
            put_rules(d, &rules)
        });
    }

    pub fn ruleremove(self: Pin<&mut Self>, index: i32) {
        self.edit(|d| {
            let mut rules = rules_of(d);
            if (0..rules.len() as i32).contains(&index) {
                rules.remove(index as usize);
            }
            put_rules(d, &rules)
        });
    }

    pub fn rulemove(self: Pin<&mut Self>, from: i32, to: i32) {
        self.edit(|d| {
            let mut rules = rules_of(d);
            let n = rules.len() as i32;
            if (0..n).contains(&from) && (0..n).contains(&to) {
                let r = rules.remove(from as usize);
                rules.insert(to as usize, r);
            }
            put_rules(d, &rules)
        });
    }

    pub fn ruleset(self: Pin<&mut Self>, index: i32, key: &QString, value: &QString) {
        let (key, value) = (key.to_string(), value.to_string());
        self.edit(|d| {
            let mut rules = rules_of(d);
            let r = rules.get_mut(index.max(0) as usize).ok_or("no such rule")?;
            let v = value.trim();
            let flag = |v: &str| match v {
                "yes" => Some(true),
                "no" => Some(false),
                _ => None,
            };
            let number = |v: &str| -> Result<Option<f64>, String> {
                if v.is_empty() {
                    Ok(None)
                } else {
                    v.parse()
                        .map(Some)
                        .map_err(|_| format!("'{v}' is not a number"))
                }
            };
            match key.as_str() {
                "name" => r.name = v.to_string(),
                "resolution" => r.when.resolution = (v != "any").then(|| v.to_string()),
                "hdr" => r.when.hdr = flag(v),
                "dv" => r.when.dolby_vision = flag(v),
                "interlaced" => r.when.interlaced = flag(v),
                "longer" => r.when.longer_than_minutes = number(v)?,
                "shorter" => r.when.shorter_than_minutes = number(v)?,
                "larger" => r.when.larger_than_gb = number(v)?,
                "settings" => r.set = unflatten(v)?,
                "filters" => {
                    let ids: Vec<String> = v
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect();
                    for f in &ids {
                        if anvil_core::scriptgen::new_block(
                            f,
                            anvil_core::scriptgen::FieldOrder::TopFieldFirst,
                        )
                        .is_none()
                        {
                            return Err(format!("unknown filter '{f}'"));
                        }
                    }
                    r.add_filters = ids;
                }
                other => return Err(format!("unknown rule part '{other}'")),
            }
            // A rule's settings must still make a valid preset.
            let mut trial = d.clone();
            preset::deep_merge(&mut trial, Value::Table(r.set.clone()));
            preset::validate(&trial).map_err(|e| format!("rule '{}': {e}", r.name))?;
            put_rules(d, &rules)
        });
    }

    pub fn save(mut self: Pin<&mut Self>, id: &QString) -> QString {
        let id = slug(&id.to_string());
        if id.is_empty() {
            self.set_message(q("Give the preset a name to save it under."));
            return QString::default();
        }
        let Some(draft) = self.rust().draft.clone() else {
            return QString::default();
        };
        // A preset cannot inherit from itself: when re-saving a user
        // preset, inherit from its own parent.
        let parent = match self.rust().parent.clone() {
            p if p == id => preset::DEFAULT_PRESET.to_string(),
            p => p,
        };
        let result = (|| -> Result<(), String> {
            let s = store().map_err(|e| e.to_string())?;
            if s.origin(&id) == Some(Origin::BuiltIn) {
                return Err(format!(
                    "'{id}' is a built-in preset; save under another name (built-ins stay as shipped)."
                ));
            }
            let text = user_preset_toml(&s, &parent, &draft).map_err(|e| e.to_string())?;
            // Check it loads before writing.
            let mut check = s.clone();
            check.add_user(&id, &text).map_err(|e| e.to_string())?;
            check.resolve(&id, None).map_err(|e| e.to_string())?;
            let dir = user_preset_dir().ok_or("no home folder")?;
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join(format!("{id}.toml")), text).map_err(|e| e.to_string())
        })();
        match result {
            Ok(()) => {
                self.as_mut().set_dirty(false);
                self.as_mut().set_message(QString::default());
                self.as_mut().set_base(q(&id));
                self.as_mut().set_suggested(q(&id));
                self.as_mut().rust_mut().parent = parent;
                q(&id)
            }
            Err(e) => {
                self.set_message(q(&e));
                QString::default()
            }
        }
    }

    pub fn optview(mut self: Pin<&mut Self>, encoder: &QString) {
        self.as_mut().set_optencoder(encoder.clone());
        self.publish();
    }

    pub fn optset(self: Pin<&mut Self>, flag: &QString, value: &QString) {
        let (flag, value) = (flag.to_string(), value.to_string());
        let kind = self.shown_encoder();
        self.edit(|d| {
            let s = schema::schema(kind);
            let args = extra_args(d, kind);
            let new = schema::set(&s, &args, &flag, &value).map_err(|e| e.to_string())?;
            put_extra_args(d, kind, new)
        });
    }

    pub fn optother(self: Pin<&mut Self>, text: &QString) {
        let text = text.to_string();
        let kind = self.shown_encoder();
        self.edit(|d| {
            let s = schema::schema(kind);
            let args = extra_args(d, kind);
            let new = schema::set_other(&s, &args, &text).map_err(|e| e.to_string())?;
            put_extra_args(d, kind, new)
        });
    }

    /// The encoder whose page is shown: the chosen one, else the draft's.
    fn shown_encoder(&self) -> EncoderKind {
        let chosen = self.optencoder().to_string();
        let from_draft = self
            .rust()
            .draft
            .as_ref()
            .and_then(|d| get_path(d, "video.encoder"))
            .and_then(Value::as_str)
            .unwrap_or("x265")
            .to_string();
        let id = if chosen.is_empty() {
            from_draft
        } else {
            chosen
        };
        Value::String(id).try_into().unwrap_or(EncoderKind::X265)
    }

    pub fn remove(mut self: Pin<&mut Self>, id: &QString) -> bool {
        let id = id.to_string();
        let Some(dir) = user_preset_dir() else {
            return false;
        };
        match store() {
            Ok(s) if s.origin(&id) == Some(Origin::User) => {}
            _ => {
                self.set_message(q("Only your own presets can be deleted."));
                return false;
            }
        }
        match std::fs::remove_file(dir.join(format!("{id}.toml"))) {
            Ok(()) => {
                self.as_mut().discard();
                true
            }
            Err(e) => {
                self.set_message(q(&format!("Could not delete: {e}")));
                false
            }
        }
    }

    /// Apply a change to the draft; keep the old draft if it fails.
    fn edit(mut self: Pin<&mut Self>, f: impl FnOnce(&mut Value) -> Result<(), String>) {
        let Some(mut draft) = self.rust().draft.clone() else {
            return;
        };
        match f(&mut draft) {
            Ok(()) => {
                self.as_mut().rust_mut().draft = Some(draft);
                self.as_mut().set_dirty(true);
                self.as_mut().set_message(QString::default());
            }
            Err(e) => self.as_mut().set_message(q(&e)),
        }
        self.publish();
    }

    fn publish(mut self: Pin<&mut Self>) {
        let Some(draft) = self.rust().draft.clone() else {
            return;
        };
        let fields = FIELDS
            .iter()
            .map(|(path, label, kind, choices, enc)| {
                format!(
                    "{path}\t{label}\t{kind}\t{}\t{choices}\t{enc}",
                    clean(&show(get_path(&draft, path)))
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let rules = rules_of(&draft)
            .iter()
            .map(|r| {
                let w = &r.when;
                [
                    clean(&r.name),
                    w.resolution.clone().unwrap_or_else(|| "any".into()),
                    yes_no(w.hdr).into(),
                    yes_no(w.dolby_vision).into(),
                    yes_no(w.interlaced).into(),
                    num(w.longer_than_minutes),
                    num(w.shorter_than_minutes),
                    num(w.larger_than_gb),
                    clean(&flatten(&r.set)),
                    r.add_filters.join(", "),
                    clean(&w.describe()),
                ]
                .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let parent = self.rust().parent.clone();
        let preview = store()
            .map_err(|e| e.to_string())
            .and_then(|s| {
                let p = if parent == self.rust().base.to_string()
                    && s.origin(&parent) == Some(Origin::User)
                {
                    preset::DEFAULT_PRESET.to_string()
                } else {
                    parent
                };
                user_preset_toml(&s, &p, &draft).map_err(|e| e.to_string())
            })
            .unwrap_or_else(|e| format!("# {e}"));
        let kind = self.shown_encoder();
        let s = schema::schema(kind);
        let (rows, other) = schema::page_rows(&s, &extra_args(&draft, kind));
        let rows = rows
            .iter()
            .map(|r| r.iter().map(|f| clean(f)).collect::<Vec<_>>().join("\t"))
            .collect::<Vec<_>>()
            .join("\n");
        self.as_mut().set_optencoder(q(schema::preset_key(kind)));
        self.as_mut().set_options(q(&rows));
        self.as_mut().set_other(q(&other));
        self.as_mut().set_fields(q(&fields));
        self.as_mut().set_rules(q(&rules));
        self.as_mut().set_preview(q(&preview));
    }
}

fn extra_args(d: &Value, kind: EncoderKind) -> Vec<String> {
    get_path(d, &format!("video.{}.extra_args", schema::preset_key(kind)))
        .cloned()
        .and_then(|a| a.try_into().ok())
        .unwrap_or_default()
}

fn put_extra_args(d: &mut Value, kind: EncoderKind, args: Vec<String>) -> Result<(), String> {
    let video = d
        .as_table_mut()
        .ok_or("preset is not a table")?
        .entry("video")
        .or_insert_with(|| Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or("video is not a table")?;
    let enc = video
        .entry(schema::preset_key(kind))
        .or_insert_with(|| Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or("encoder settings are not a table")?;
    enc.insert(
        "extra_args".into(),
        Value::Array(args.into_iter().map(Value::String).collect()),
    );
    Ok(())
}

fn put_rules(d: &mut Value, rules: &[Rule]) -> Result<(), String> {
    let v = Value::try_from(rules).map_err(|e| e.to_string())?;
    d.as_table_mut()
        .ok_or("preset is not a table")?
        .insert("rules".into(), v);
    Ok(())
}
