// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! App-wide settings in `~/.config/anvil/settings.toml`, shared by the GUI,
//! the CLI and the install script.
//!
//! - `forge`: Forge mode (metalworking names in the interface).
//! - `hardware_encoders`: whether GPU encoders are used at all. The install
//!   script sets it to false when you choose CPU encoding only; GPU presets
//!   are then hidden and no GPU checks run.

use std::path::PathBuf;

use crate::encoder::EncoderKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub forge: bool,
    pub hardware_encoders: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            forge: false,
            hardware_encoders: true,
        }
    }
}

pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("anvil").join("settings.toml"))
}

impl Settings {
    pub fn parse(text: &str) -> Settings {
        let t = text.parse::<toml::Table>().unwrap_or_default();
        let flag = |k: &str, d: bool| t.get(k).and_then(toml::Value::as_bool).unwrap_or(d);
        let d = Settings::default();
        Settings {
            forge: flag("forge", d.forge),
            hardware_encoders: flag("hardware_encoders", d.hardware_encoders),
        }
    }

    pub fn load() -> Settings {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| Settings::parse(&t))
            .unwrap_or_default()
    }

    pub fn to_toml(&self) -> String {
        format!(
            "# Anvil settings\n\
             # Forge mode: metalworking names in the interface.\n\
             forge = {}\n\
             # GPU encoders (NVIDIA, Intel, AMD, VAAPI). false = CPU encoding only.\n\
             hardware_encoders = {}\n",
            self.forge, self.hardware_encoders
        )
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(p) = path() else { return Ok(()) };
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(p, self.to_toml())
    }
}

/// The program an encoder runs, for checking it is installed.
pub fn encoder_program(kind: EncoderKind) -> &'static str {
    match kind {
        EncoderKind::X265 => "x265",
        EncoderKind::X264 => "x264",
        EncoderKind::SvtAv1 => "SvtAv1EncApp",
        EncoderKind::Rav1e => "rav1e",
        EncoderKind::NvEncC => "NVEncC",
        EncoderKind::QsvEncC => "QSVEncC",
        EncoderKind::VceEncC => "VCEEncC",
        EncoderKind::FfmpegVaapi => "ffmpeg",
    }
}

/// PCI vendor ids of the graphics cards present, from /sys/class/drm
/// (`0x10de` NVIDIA, `0x8086` Intel, `0x1002` AMD).
pub fn gpu_vendors() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("/sys/class/drm")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with("card") && !n.contains('-')
        })
        .filter_map(|e| std::fs::read_to_string(e.path().join("device/vendor")).ok())
        .map(|s| s.trim().to_ascii_lowercase())
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The graphics card a GPU encoder needs, as (PCI vendor id, name).
fn needed_gpu(kind: EncoderKind) -> Option<(&'static str, &'static str)> {
    match kind {
        EncoderKind::NvEncC => Some(("0x10de", "an NVIDIA graphics card")),
        EncoderKind::QsvEncC => Some(("0x8086", "Intel graphics")),
        EncoderKind::VceEncC => Some(("0x1002", "AMD Radeon graphics")),
        _ => None,
    }
}

/// Can this machine use the encoder? `Err` says why not. Quick: looks for
/// the program (and, for VAAPI, a GPU render node); it does not start it.
pub fn encoder_available(kind: EncoderKind, settings: &Settings) -> Result<(), String> {
    if kind.is_hardware() && !settings.hardware_encoders {
        return Err("GPU encoders are off (CPU encoding only)".into());
    }
    if kind == EncoderKind::FfmpegVaapi && !std::path::Path::new("/dev/dri/renderD128").exists() {
        return Err("no GPU render node for VAAPI".into());
    }
    // An encoder for a graphics card this machine does not have (VCEEncC on
    // a Ryzen without integrated graphics, say) is useless even if installed.
    if let Some((vendor, name)) = needed_gpu(kind) {
        let vendors = gpu_vendors();
        if !vendors.is_empty() && !vendors.iter().any(|v| v == vendor) {
            return Err(format!("this machine has no {name}"));
        }
    }
    let program = encoder_program(kind);
    if crate::tools::which(program).is_none() {
        return Err(format!("{} is not installed", kind.label()));
    }
    if kind.is_hardware() {
        return probe(kind);
    }
    Ok(())
}

type ProbeCache = std::sync::Mutex<std::collections::HashMap<EncoderKind, Result<(), String>>>;
static PROBES: std::sync::OnceLock<ProbeCache> = std::sync::OnceLock::new();

/// Forget the cached GPU checks (after installing something, "Check again").
pub fn forget_probes() {
    if let Some(m) = PROBES.get() {
        if let Ok(mut m) = m.lock() {
            m.clear();
        }
    }
}

/// A GPU encoder that is installed may still not work: an AUR build whose
/// libraries were since updated fails to start ("error while loading shared
/// libraries"), and a VAAPI driver may only decode (NVIDIA's does). Checked
/// once per run, quickly, and remembered.
fn probe(kind: EncoderKind) -> Result<(), String> {
    let cache = PROBES.get_or_init(Default::default);
    if let Some(r) = cache.lock().ok().and_then(|m| m.get(&kind).cloned()) {
        return r;
    }
    let r = if kind == EncoderKind::FfmpegVaapi {
        probe_vaapi()
    } else {
        probe_starts(kind)
    };
    if let Ok(mut m) = cache.lock() {
        m.insert(kind, r.clone());
    }
    r
}

fn probe_starts(kind: EncoderKind) -> Result<(), String> {
    let program = crate::tools::program(encoder_program(kind));
    let out = crate::process::run_with_timeout(
        &program,
        &["--version".to_string()],
        std::time::Duration::from_secs(10),
    )
    .map_err(|e| format!("{program} does not start: {e}"))?;
    let text = format!("{}\n{}", out.stdout, out.stderr);
    if let Some(line) = text
        .lines()
        .find(|l| l.contains("error while loading shared libraries"))
    {
        let lib = line
            .split("libraries: ")
            .nth(1)
            .and_then(|r| r.split(':').next())
            .unwrap_or("a library");
        let pkg = match kind {
            EncoderKind::NvEncC => "nvenc",
            EncoderKind::QsvEncC => "qsvenc",
            _ => "vceenc",
        };
        return Err(format!(
            "{} is installed but cannot start: {lib} is missing (a library it was built against has changed). Rebuild it: paru -S --rebuild {pkg}",
            kind.label()
        ));
    }
    Ok(())
}

fn probe_vaapi() -> Result<(), String> {
    let reports = crate::hw::detect("/dev/dri/renderD128", std::time::Duration::from_secs(10));
    let Some(r) = reports.iter().find(|r| r.kind == EncoderKind::FfmpegVaapi) else {
        return Err("VAAPI could not be checked".into());
    };
    if !r.installed {
        return Err("vainfo is not installed, so VAAPI encoding cannot be checked (sudo pacman -S libva-utils)".into());
    }
    if !r.codecs.iter().any(|c| c == "hevc10" || c == "av1") {
        return Err(if r.codecs.is_empty() {
            "the graphics driver offers no VAAPI encoding (NVIDIA's VAAPI driver only decodes; use Fast (NVIDIA))".into()
        } else {
            "the graphics driver cannot encode 10-bit HEVC or AV1 with VAAPI".into()
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_defaults() {
        assert_eq!(Settings::parse(""), Settings::default());
        let s = Settings::parse("forge = true\nhardware_encoders = false\n");
        assert!(s.forge && !s.hardware_encoders);
        assert_eq!(Settings::parse(&s.to_toml()), s);
        assert!(Settings::parse("forge = \"yes\"").hardware_encoders);
    }

    #[test]
    fn cpu_only_hides_gpu_encoders() {
        let cpu = Settings {
            hardware_encoders: false,
            ..Default::default()
        };
        assert!(encoder_available(EncoderKind::NvEncC, &cpu)
            .unwrap_err()
            .contains("CPU encoding only"));
        // A CPU encoder is judged only by whether it is installed.
        let r = encoder_available(EncoderKind::X265, &cpu);
        assert_eq!(r.is_ok(), crate::tools::which("x265").is_some());
    }
}
