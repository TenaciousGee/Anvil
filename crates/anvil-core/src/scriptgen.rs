// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! VapourSynth filter profiles rendered to `.vpy` scripts.
//!
//! A [`FilterProfile`] is an ordered list of [`FilterBlock`]s with plain
//! parameters, so the UI can show sliders and toggles instead of code. The
//! same script drives both the preview (via VSScript) and the encode (via
//! `vspipe`), so what you preview is what you get.

use serde::{Deserialize, Serialize};

use crate::crop::Crop;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterProfile {
    pub name: String,
    pub source: SourceFilter,
    #[serde(default)]
    pub blocks: Vec<FilterBlock>,
    /// A hand-edited script used instead of the generated one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
}

impl Default for FilterProfile {
    fn default() -> Self {
        FilterProfile {
            name: "Clean (crop only)".into(),
            source: SourceFilter::BestSource,
            blocks: Vec::new(),
            script: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFilter {
    /// `bs.VideoSource`: accurate, fast seeking. Default.
    BestSource,
    /// `lsmas.LWLibavSource`: fallback for awkward files.
    LSmash,
    /// `d2v.Source`: DVD MPEG-2 via a D2V project file.
    D2v,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    Light,
    Medium,
    Strong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldOrder {
    TopFieldFirst,
    BottomFieldFirst,
}

impl FieldOrder {
    fn tff(self) -> bool {
        matches!(self, FieldOrder::TopFieldFirst)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FilterBlock {
    /// Inverse telecine for film-based NTSC DVDs (VFM + VDecimate).
    Ivtc {
        field_order: FieldOrder,
    },
    /// Deinterlacing with BWDIF: same frame rate, fast, no Python
    /// dependencies. The default for interlaced sources.
    Deinterlace {
        field_order: FieldOrder,
    },
    /// Slower, higher-quality deinterlacing (QTGMC via havsfunc).
    Qtgmc {
        field_order: FieldOrder,
        preset: String,
    },
    Denoise {
        strength: Strength,
    },
    Deband {
        strength: Strength,
    },
    Resize {
        width: u32,
        height: u32,
    },
    /// Contrast-adaptive sharpening (CAS).
    Sharpen {
        strength: Strength,
    },
    /// Fine, steady film grain: hides banding and keeps a filmic look
    /// without the bitrate cost of moving grain.
    Grain {
        strength: Strength,
    },
    /// Raw VapourSynth Python inserted as-is; `clip` is the working clip.
    Custom {
        label: String,
        code: String,
    },
}

impl FilterBlock {
    pub fn label(&self) -> String {
        match self {
            FilterBlock::Ivtc { .. } => "Inverse telecine".into(),
            FilterBlock::Deinterlace { .. } => "Deinterlace (BWDIF)".into(),
            FilterBlock::Qtgmc { preset, .. } => format!("Deinterlace (QTGMC {preset})"),
            FilterBlock::Denoise { strength } => format!("Denoise ({strength:?})"),
            FilterBlock::Deband { strength } => format!("Deband ({strength:?})"),
            FilterBlock::Resize { width, height } => format!("Resize to {width}x{height}"),
            FilterBlock::Sharpen { strength } => format!("Sharpen ({strength:?})"),
            FilterBlock::Grain { strength } => format!("Film grain ({strength:?})"),
            FilterBlock::Custom { label, .. } => label.clone(),
        }
    }

    /// True for blocks that must run before cropping (field-based filters).
    pub fn runs_before_crop(&self) -> bool {
        matches!(
            self,
            FilterBlock::Ivtc { .. } | FilterBlock::Qtgmc { .. } | FilterBlock::Deinterlace { .. }
        )
    }

    /// True when the block changes the frame count, which invalidates VFR
    /// timecodes and per-frame HDR metadata.
    pub fn changes_frame_count(&self) -> bool {
        matches!(self, FilterBlock::Ivtc { .. } | FilterBlock::Qtgmc { .. })
    }
}

/// A plugin or Python module a profile needs, for the plugin detector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Requirement {
    Plugin(&'static str),
    PythonModule(&'static str),
}

impl FilterProfile {
    pub fn requirements(&self) -> Vec<Requirement> {
        let mut req = vec![match self.source {
            SourceFilter::BestSource => Requirement::Plugin("bs"),
            SourceFilter::LSmash => Requirement::Plugin("lsmas"),
            SourceFilter::D2v => Requirement::Plugin("d2v"),
        }];
        for b in &self.blocks {
            match b {
                FilterBlock::Ivtc { .. } => req.push(Requirement::Plugin("vivtc")),
                FilterBlock::Deinterlace { .. } => req.push(Requirement::Plugin("bwdif")),
                FilterBlock::Qtgmc { .. } => req.push(Requirement::PythonModule("havsfunc")),
                FilterBlock::Denoise { .. } | FilterBlock::Deband { .. } => {
                    req.push(Requirement::Plugin("vszip"))
                }
                FilterBlock::Sharpen { .. } => req.push(Requirement::Plugin("cas")),
                FilterBlock::Grain { .. } => req.push(Requirement::Plugin("noise")),
                FilterBlock::Resize { .. } | FilterBlock::Custom { .. } => {}
            }
        }
        req.sort();
        req.dedup();
        req
    }

    pub fn changes_frame_count(&self) -> bool {
        self.blocks.iter().any(FilterBlock::changes_frame_count)
    }
}

/// Job-specific inputs to the script.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptContext {
    pub source_path: String,
    pub crop: Crop,
    /// Bit depth handed to the encoder (10 for the Quality Default).
    pub output_bit_depth: u8,
}

/// Render a profile to VapourSynth Python (or return its hand-edited script).
pub fn render(profile: &FilterProfile, ctx: &ScriptContext) -> String {
    if let Some(script) = &profile.script {
        return script.clone();
    }
    render_generated(profile, ctx)
}

/// The generated script, ignoring any hand edit.
pub fn render_generated(profile: &FilterProfile, ctx: &ScriptContext) -> String {
    let mut s = String::new();
    s.push_str("# Generated by Anvil. Edit the filter profile rather than this file,\n");
    s.push_str("# or switch the job to 'custom script' to take over by hand.\n");
    s.push_str("import vapoursynth as vs\n");
    if profile
        .blocks
        .iter()
        .any(|b| matches!(b, FilterBlock::Qtgmc { .. }))
    {
        s.push_str("import havsfunc as haf\n");
    }
    s.push_str("core = vs.core\n\n");

    let src = py_str(&ctx.source_path);
    let source_line = match profile.source {
        SourceFilter::BestSource => format!("clip = core.bs.VideoSource(source={src})\n"),
        SourceFilter::LSmash => format!("clip = core.lsmas.LWLibavSource(source={src})\n"),
        SourceFilter::D2v => format!("clip = core.d2v.Source(input={src}, rff=False)\n"),
    };
    s.push_str(&source_line);

    for b in profile.blocks.iter().filter(|b| b.runs_before_crop()) {
        push_block(&mut s, b);
    }

    if !ctx.crop.is_none() {
        let c = ctx.crop;
        s.push_str(&format!(
            "\n# Crop\nclip = core.std.Crop(clip, left={}, right={}, top={}, bottom={})\n",
            c.left, c.right, c.top, c.bottom
        ));
    }

    for b in profile.blocks.iter().filter(|b| !b.runs_before_crop()) {
        push_block(&mut s, b);
    }

    s.push_str(&format!(
        "\n# Hand the encoder {bits}-bit video (dither only when reducing depth)\n\
         if clip.format.bits_per_sample != {bits} or clip.format.sample_type != vs.INTEGER:\n    \
         clip = core.resize.Point(clip, format=clip.format.replace(bits_per_sample={bits}, sample_type=vs.INTEGER),\n                             \
         dither_type='error_diffusion' if clip.format.bits_per_sample > {bits} else 'none')\n\n\
         clip.set_output()\n",
        bits = ctx.output_bit_depth
    ));
    s
}

fn push_block(s: &mut String, b: &FilterBlock) {
    s.push_str(&format!("\n# {}\n", b.label()));
    match b {
        FilterBlock::Ivtc { field_order } => {
            s.push_str(&format!(
                "clip = core.vivtc.VFM(clip, order={})\nclip = core.vivtc.VDecimate(clip)\n",
                u8::from(field_order.tff())
            ));
        }
        FilterBlock::Deinterlace { field_order } => {
            // field 1 = top field first, 0 = bottom; same frame rate.
            s.push_str(&format!(
                "clip = core.bwdif.Bwdif(clip, field={})\n",
                u8::from(field_order.tff())
            ));
        }
        FilterBlock::Qtgmc {
            field_order,
            preset,
        } => {
            s.push_str(&format!(
                "clip = haf.QTGMC(clip, Preset={}, TFF={}, FPSDivisor=2)\n",
                py_str(preset),
                if field_order.tff() { "True" } else { "False" }
            ));
        }
        // vszip (VapourSynth API 4). The older DFTTest and neo_f3kdb
        // plugins use API 3, which VapourSynth R80 and later no longer load.
        FilterBlock::Denoise { strength } => {
            // Non-local means, KNLMeansCL's algorithm on the CPU; h is on
            // the 8-bit scale at every bit depth.
            let h = match strength {
                Strength::Light => 0.6,
                Strength::Medium => 1.0,
                Strength::Strong => 1.6,
            };
            s.push_str(&format!(
                "clip = core.vszip.NLMeans(clip, d=1, a=2, s=4, h={h}, channels=\"Y\")\n\
                 clip = core.vszip.NLMeans(clip, d=1, a=2, s=4, h={h}, channels=\"UV\")\n"
            ));
        }
        FilterBlock::Deband { strength } => {
            // f3kdb thresholds on the 8-bit scale (luma, chroma), worked in
            // 16-bit so the smoothed gradients keep their precision.
            let (y, c) = match strength {
                Strength::Light => (0.5, 0.375),
                Strength::Medium => (0.75, 0.5),
                Strength::Strong => (1.0, 0.75),
            };
            s.push_str(&format!(
                "clip = core.resize.Point(clip, format=clip.format.replace(bits_per_sample=16))\n\
                 clip = core.vszip.Deband(clip, thr=[{y}, {c}], grain=[0, 0])\n"
            ));
        }
        FilterBlock::Resize { width, height } => {
            s.push_str(&format!(
                "clip = core.resize.Spline36(clip, width={width}, height={height})\n"
            ));
        }
        FilterBlock::Sharpen { strength } => {
            let k = match strength {
                Strength::Light => 0.3,
                Strength::Medium => 0.5,
                Strength::Strong => 0.7,
            };
            s.push_str(&format!("clip = core.cas.CAS(clip, sharpness={k})\n"));
        }
        FilterBlock::Grain { strength } => {
            let var = match strength {
                Strength::Light => 0.4,
                Strength::Medium => 0.8,
                Strength::Strong => 1.6,
            };
            s.push_str(&format!(
                "clip = core.noise.Add(clip, var={var}, uvar=0, constant=True)\n"
            ));
        }
        FilterBlock::Custom { code, .. } => {
            s.push_str(code.trim_end());
            s.push('\n');
        }
    }
}

/// Blocks the editor offers: id, label and what it is for.
pub const CATALOGUE: &[(&str, &str, &str)] = &[
    (
        "deinterlace",
        "Deinterlace (BWDIF)",
        "For true interlaced video (TV, concerts, some DVD extras). Fast and clean.",
    ),
    (
        "qtgmc",
        "Deinterlace (QTGMC)",
        "Slower and finer than BWDIF. Needs havsfunc and vsjetpack (Python, advanced).",
    ),
    (
        "ivtc",
        "Inverse telecine",
        "For film-based NTSC DVDs: restores the original 23.976 fps frames.",
    ),
    (
        "denoise",
        "Denoise",
        "Calms grain and noise (non-local means, vszip). Saves bitrate; strong settings soften detail.",
    ),
    (
        "deband",
        "Deband",
        "Smooths banding in skies and gradients (f3kdb-style, vszip).",
    ),
    (
        "sharpen",
        "Sharpen",
        "Contrast-adaptive sharpening (CAS), gentle on edges.",
    ),
    (
        "grain",
        "Film grain",
        "Adds fine, steady grain that hides banding at little bitrate cost.",
    ),
    (
        "resize",
        "Resize",
        "Scale to a new size, such as 1920x1080 from UHD.",
    ),
    (
        "custom",
        "Custom VapourSynth",
        "Your own lines; `clip` is the working clip.",
    ),
];

/// A new block of the given catalogue id with sensible settings.
pub fn new_block(id: &str, field_order: FieldOrder) -> Option<FilterBlock> {
    Some(match id {
        "qtgmc" => FilterBlock::Qtgmc {
            field_order,
            preset: "Slower".into(),
        },
        "ivtc" => FilterBlock::Ivtc { field_order },
        "deinterlace" => FilterBlock::Deinterlace { field_order },
        "denoise" => FilterBlock::Denoise {
            strength: Strength::Light,
        },
        "deband" => FilterBlock::Deband {
            strength: Strength::Light,
        },
        "sharpen" => FilterBlock::Sharpen {
            strength: Strength::Light,
        },
        "grain" => FilterBlock::Grain {
            strength: Strength::Light,
        },
        "resize" => FilterBlock::Resize {
            width: 1920,
            height: 1080,
        },
        "custom" => FilterBlock::Custom {
            label: "Custom".into(),
            code: "# clip = core.std.Levels(clip, gamma=1.05)".into(),
        },
        _ => return None,
    })
}

/// QTGMC presets, slowest (best) to fastest.
pub const QTGMC_PRESETS: &[&str] = &[
    "Placebo",
    "Very Slow",
    "Slower",
    "Slow",
    "Medium",
    "Fast",
    "Faster",
    "Very Fast",
    "Super Fast",
    "Ultra Fast",
    "Draft",
];

impl FilterBlock {
    /// The editor's id for this block.
    pub fn id(&self) -> &'static str {
        match self {
            FilterBlock::Qtgmc { .. } => "qtgmc",
            FilterBlock::Ivtc { .. } => "ivtc",
            FilterBlock::Deinterlace { .. } => "deinterlace",
            FilterBlock::Denoise { .. } => "denoise",
            FilterBlock::Deband { .. } => "deband",
            FilterBlock::Sharpen { .. } => "sharpen",
            FilterBlock::Grain { .. } => "grain",
            FilterBlock::Resize { .. } => "resize",
            FilterBlock::Custom { .. } => "custom",
        }
    }

    /// Change one setting: strength (light, medium, strong), preset (QTGMC),
    /// width, height, label or code (custom).
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let strength = || match value {
            "light" => Ok(Strength::Light),
            "medium" => Ok(Strength::Medium),
            "strong" => Ok(Strength::Strong),
            _ => Err(format!("'{value}' is not light, medium or strong")),
        };
        let size = || {
            value
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|v| (64..=8192).contains(v) && v % 2 == 0)
                .ok_or_else(|| format!("'{value}' is not an even size between 64 and 8192"))
        };
        match (self, key) {
            (
                FilterBlock::Denoise { strength: s }
                | FilterBlock::Deband { strength: s }
                | FilterBlock::Sharpen { strength: s }
                | FilterBlock::Grain { strength: s },
                "strength",
            ) => *s = strength()?,
            (FilterBlock::Qtgmc { preset, .. }, "preset") => {
                if !QTGMC_PRESETS.contains(&value) {
                    return Err(format!("'{value}' is not a QTGMC preset"));
                }
                *preset = value.into();
            }
            (FilterBlock::Resize { width, .. }, "width") => *width = size()?,
            (FilterBlock::Resize { height, .. }, "height") => *height = size()?,
            (FilterBlock::Custom { label, .. }, "label") => *label = value.trim().to_string(),
            (FilterBlock::Custom { code, .. }, "code") => *code = value.to_string(),
            (b, k) => return Err(format!("{} has no setting '{k}'", b.label())),
        }
        Ok(())
    }

    /// The editable value of a setting, for the editor.
    pub fn setting(&self) -> String {
        match self {
            FilterBlock::Denoise { strength }
            | FilterBlock::Deband { strength }
            | FilterBlock::Sharpen { strength }
            | FilterBlock::Grain { strength } => format!("{strength:?}").to_lowercase(),
            FilterBlock::Qtgmc { preset, .. } => preset.clone(),
            FilterBlock::Resize { width, height } => format!("{width}x{height}"),
            FilterBlock::Custom { code, .. } => code.clone(),
            FilterBlock::Ivtc { .. } | FilterBlock::Deinterlace { .. } => String::new(),
        }
    }
}

impl Requirement {
    pub fn name(&self) -> &'static str {
        match self {
            Requirement::Plugin(n) | Requirement::PythonModule(n) => n,
        }
    }

    /// What to install on CachyOS / Arch to get it.
    pub fn install_hint(&self) -> &'static str {
        if std::env::var_os("FLATPAK_ID").is_some() {
            return "should be bundled in the Flatpak: please report it as a bug";
        }
        match self.name() {
            "havsfunc" => {
                "QTGMC needs the Python packages havsfunc and vsjetpack (pip install --user havsfunc). The Deinterlace (BWDIF) block needs nothing extra"
            }
            "bs" | "lsmas" | "d2v" | "vivtc" | "bwdif" | "vszip" | "cas" | "noise" | "mv" | "znedi3" => {
                "run ./install-cachyos.sh --with-tools, or python3 packaging/scripts/install-vs-plugins.py"
            }
            _ => "see the VapourSynth plugin list on PyPI (pip packages named vapoursynth-...)",
        }
    }
}

/// What the local VapourSynth has: plugin namespaces and Python modules.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Inventory {
    pub version: String,
    pub plugins: std::collections::BTreeSet<String>,
    pub modules: std::collections::BTreeSet<String>,
}

impl Inventory {
    pub fn has(&self, r: &Requirement) -> bool {
        match r {
            Requirement::Plugin(n) => self.plugins.contains(*n),
            Requirement::PythonModule(n) => self.modules.contains(*n),
        }
    }
}

/// Ask Python's VapourSynth what is installed.
pub fn inventory(timeout: std::time::Duration) -> Result<Inventory, String> {
    let script = "import json, importlib.util as u\n\
try:\n    import vapoursynth as vs\n    plugins = sorted(p.namespace for p in vs.core.plugins()); version = str(vs.__version__)\n\
except Exception as e:\n    plugins = None; version = str(e)\n\
mods = [m for m in ('havsfunc', 'vsutil', 'mvsfunc') if u.find_spec(m)]\n\
print(json.dumps({'plugins': plugins, 'modules': mods, 'version': version}))";
    let out = crate::process::run_with_timeout(
        "python3",
        &["-c".to_string(), script.to_string()],
        timeout,
    )
    .map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(out.stdout.trim())
        .map_err(|_| format!("python3 said: {}", out.stderr.trim()))?;
    let list = |k: &str| {
        v.get(k).and_then(|x| x.as_array()).map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect()
        })
    };
    let plugins = list("plugins").ok_or_else(|| {
        format!(
            "VapourSynth's Python module is not available ({})",
            v.get("version").and_then(|x| x.as_str()).unwrap_or("?")
        )
    })?;
    Ok(Inventory {
        version: v
            .get("version")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .into(),
        plugins,
        modules: list("modules").unwrap_or_default(),
    })
}

/// A Python string literal for any path, including quotes and backslashes.
pub fn py_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(crop: Crop) -> ScriptContext {
        ScriptContext {
            source_path: "/films/It's \"Quoted\".mkv".into(),
            crop,
            output_bit_depth: 10,
        }
    }

    #[test]
    fn default_profile_is_source_crop_and_depth() {
        let crop = Crop {
            top: 140,
            bottom: 140,
            ..Default::default()
        };
        let s = render(&FilterProfile::default(), &ctx(crop));
        assert!(s.contains(r#"core.bs.VideoSource(source="/films/It's \"Quoted\".mkv")"#));
        assert!(s.contains("core.std.Crop(clip, left=0, right=0, top=140, bottom=140)"));
        assert!(s.contains("bits_per_sample=10"));
        assert!(s.trim_end().ends_with("clip.set_output()"));
        assert!(!s.contains("havsfunc"));
    }

    #[test]
    fn field_filters_run_before_crop() {
        let profile = FilterProfile {
            name: "DVD film".into(),
            source: SourceFilter::LSmash,
            blocks: vec![
                FilterBlock::Deband {
                    strength: Strength::Light,
                },
                FilterBlock::Ivtc {
                    field_order: FieldOrder::TopFieldFirst,
                },
            ],
            script: None,
        };
        let s = render(
            &profile,
            &ctx(Crop {
                left: 8,
                right: 8,
                ..Default::default()
            }),
        );
        let ivtc = s.find("VFM").unwrap();
        let crop = s.find("std.Crop").unwrap();
        let deband = s.find("vszip.Deband").unwrap();
        assert!(ivtc < crop && crop < deband);
        assert!(profile.changes_frame_count());
        assert_eq!(
            profile.requirements(),
            vec![
                Requirement::Plugin("lsmas"),
                Requirement::Plugin("vivtc"),
                Requirement::Plugin("vszip")
            ]
        );
    }

    #[test]
    fn profile_round_trips_through_toml() {
        let profile = FilterProfile {
            name: "Interlaced".into(),
            source: SourceFilter::BestSource,
            blocks: vec![FilterBlock::Qtgmc {
                field_order: FieldOrder::TopFieldFirst,
                preset: "Slower".into(),
            }],
            script: None,
        };
        let text = toml::to_string(&profile).unwrap();
        let back: FilterProfile = toml::from_str(&text).unwrap();
        assert_eq!(back, profile);
    }

    #[test]
    fn editing_blocks_and_hand_scripts() {
        let mut b = new_block("denoise", FieldOrder::TopFieldFirst).unwrap();
        assert_eq!(b.setting(), "light");
        b.set("strength", "strong").unwrap();
        assert_eq!(b.label(), "Denoise (Strong)");
        assert!(b.set("strength", "huge").is_err());
        assert!(b.set("width", "1920").is_err(), "denoise has no width");
        let mut r = new_block("resize", FieldOrder::TopFieldFirst).unwrap();
        r.set("width", "1280").unwrap();
        r.set("height", "720").unwrap();
        assert_eq!(r.setting(), "1280x720");
        assert!(r.set("height", "721").is_err(), "odd sizes are refused");
        let mut q = new_block("qtgmc", FieldOrder::BottomFieldFirst).unwrap();
        q.set("preset", "Very Slow").unwrap();
        assert!(q.set("preset", "Turbo").is_err());
        assert!(new_block("magic", FieldOrder::TopFieldFirst).is_none());
        assert!(CATALOGUE
            .iter()
            .all(|(id, _, _)| new_block(id, FieldOrder::TopFieldFirst).is_some()));

        let mut p = FilterProfile::default();
        p.blocks
            .push(new_block("sharpen", FieldOrder::TopFieldFirst).unwrap());
        p.blocks
            .push(new_block("grain", FieldOrder::TopFieldFirst).unwrap());
        let ctx = ScriptContext {
            source_path: "/m/a.mkv".into(),
            crop: Crop::default(),
            output_bit_depth: 10,
        };
        let s = render(&p, &ctx);
        assert!(s.contains("core.cas.CAS(clip, sharpness=0.3)"));
        assert!(s.contains("core.noise.Add(clip, var=0.4, uvar=0, constant=True)"));
        assert!(p.requirements().contains(&Requirement::Plugin("cas")));
        assert!(Requirement::Plugin("vszip")
            .install_hint()
            .contains("install-cachyos.sh"));
        p.script = Some("# mine\n".into());
        assert_eq!(render(&p, &ctx), "# mine\n");
        assert_eq!(render_generated(&p, &ctx), s);
    }

    #[test]
    fn inventory_of_this_machine() {
        // Only meaningful where VapourSynth is installed.
        if let Ok(inv) = inventory(std::time::Duration::from_secs(30)) {
            assert!(inv.plugins.contains("std"), "{inv:?}");
            assert!(inv.has(&Requirement::Plugin("std")));
            assert!(!inv.has(&Requirement::Plugin("no_such_plugin")));
        }
    }
}
