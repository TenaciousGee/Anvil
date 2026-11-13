// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Hardware encoder detection: which GPU encoders work on this machine and
//! which codecs they offer.
//!
//! rigaya's encoders (NVEncC, QSVEncC, VCEEncC) answer `--check-hw` with
//! the device and its codecs and exit non-zero when there is no usable
//! device. VAAPI is read from `vainfo`: a profile with an `EncSlice` (or
//! low-power `EncSliceLP`) entry point can encode.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::encoder::EncoderKind;
use crate::process::run_with_timeout;
use crate::tools::which as find_in_path;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HwReport {
    pub kind: EncoderKind,
    /// The program was found.
    pub installed: bool,
    /// A device answered and can encode.
    pub usable: bool,
    /// Device name as reported, if any.
    pub device: String,
    /// "hevc", "av1", "h264"; "hevc10" when 10-bit HEVC is confirmed.
    pub codecs: Vec<String>,
    /// Why it is not usable, or a note.
    pub note: String,
}

impl HwReport {
    pub fn summary(&self) -> String {
        if !self.installed {
            return self.note.clone();
        }
        if !self.usable {
            return format!("Installed, but no usable device: {}", self.note);
        }
        let codecs: Vec<String> = self
            .codecs
            .iter()
            .filter(|c| *c != "hevc10")
            .map(|c| match c.as_str() {
                "hevc" if self.codecs.iter().any(|x| x == "hevc10") => "HEVC 10-bit".to_string(),
                "hevc" => "HEVC".into(),
                "av1" => "AV1".into(),
                "h264" => "H.264".into(),
                other => other.to_uppercase(),
            })
            .collect();
        let dev = if self.device.is_empty() {
            String::new()
        } else {
            format!("{}: ", self.device)
        };
        format!("{dev}{}", codecs.join(", "))
    }
}

/// Codecs named in a rigaya `--check-hw` / `--check-features` answer.
pub fn parse_rigaya(out: &str) -> (String, Vec<String>) {
    let mut codecs = Vec::new();
    let mut device = String::new();
    for line in out.lines().map(str::trim) {
        let lower = line.to_ascii_lowercase();
        if device.is_empty()
            && (line.starts_with('#') || lower.starts_with("device") || lower.contains("gpu #"))
        {
            device = line
                .trim_start_matches('#')
                .split_once(':')
                .map(|(_, d)| d)
                .unwrap_or(line)
                .split(['(', '['])
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
        }
        let mut add = |c: &str| {
            if !codecs.iter().any(|x| x == c) {
                codecs.push(c.to_string());
            }
        };
        if lower.contains("h.264") || lower.contains("avc") {
            add("h264");
        }
        if lower.contains("hevc") || lower.contains("h.265") {
            add("hevc");
            if lower.contains("10bit") || lower.contains("10-bit") || lower.contains("main10") {
                add("hevc10");
            }
        }
        if lower.contains("av1") {
            add("av1");
        }
    }
    (device, codecs)
}

/// Encoding codecs and the driver from `vainfo` output.
pub fn parse_vainfo(out: &str) -> (String, Vec<String>) {
    let mut codecs = Vec::new();
    let mut driver = String::new();
    for line in out.lines().map(str::trim) {
        if let Some(d) = line.split_once("Driver version:").map(|(_, d)| d.trim()) {
            driver = d.to_string();
        }
        let Some((profile, entry)) = line.split_once(':') else {
            continue;
        };
        let (profile, entry) = (profile.trim(), entry.trim());
        if !profile.starts_with("VAProfile") || !entry.starts_with("VAEntrypointEncSlice") {
            continue;
        }
        let codec = match profile {
            p if p.starts_with("VAProfileH264") => "h264",
            "VAProfileHEVCMain" => "hevc",
            "VAProfileHEVCMain10" => "hevc10",
            p if p.starts_with("VAProfileAV1") => "av1",
            _ => continue,
        };
        for c in [codec, if codec == "hevc10" { "hevc" } else { codec }] {
            if !codecs.iter().any(|x| x == c) {
                codecs.push(c.to_string());
            }
        }
    }
    (driver, codecs)
}

fn rigaya(kind: EncoderKind, program: &str, timeout: Duration) -> HwReport {
    let mut r = HwReport {
        kind,
        installed: find_in_path(program).is_some(),
        usable: false,
        device: String::new(),
        codecs: Vec::new(),
        note: String::new(),
    };
    if !r.installed {
        r.note = format!("{program} is not installed");
        return r;
    }
    match run_with_timeout(
        &crate::tools::program(program),
        &["--check-hw".to_string()],
        timeout,
    ) {
        Ok(out) => {
            let text = format!("{}\n{}", out.stdout, out.stderr);
            let (device, codecs) = parse_rigaya(&text);
            r.device = device;
            r.codecs = codecs;
            r.usable = out.success && !r.codecs.is_empty();
            if !r.usable {
                r.note = text
                    .lines()
                    .map(str::trim)
                    .rfind(|l| !l.is_empty())
                    .unwrap_or("no device found")
                    .to_string();
            }
        }
        Err(e) => r.note = e.to_string(),
    }
    r
}

fn vaapi(device: &str, timeout: Duration) -> HwReport {
    let mut r = HwReport {
        kind: EncoderKind::FfmpegVaapi,
        installed: find_in_path("vainfo").is_some(),
        usable: false,
        device: device.to_string(),
        codecs: Vec::new(),
        note: String::new(),
    };
    if !Path::new(device).exists() {
        r.note = format!("{device} does not exist (no GPU render node)");
        return r;
    }
    if !r.installed {
        r.note = "vainfo is not installed (package libva-utils)".into();
        return r;
    }
    let args: Vec<String> = ["--display", "drm", "--device", device]
        .iter()
        .map(|s| s.to_string())
        .collect();
    match run_with_timeout("vainfo", &args, timeout) {
        Ok(out) => {
            let (driver, codecs) = parse_vainfo(&format!("{}\n{}", out.stdout, out.stderr));
            if !driver.is_empty() {
                r.device = format!("{device} ({driver})");
            }
            r.codecs = codecs;
            r.usable = !r.codecs.is_empty();
            if !r.usable {
                r.note = "the driver offers no encoding profiles".into();
            }
        }
        Err(e) => r.note = e.to_string(),
    }
    r
}

/// Check every hardware encoder. Each check is bounded by `timeout`.
pub fn detect(vaapi_device: &str, timeout: Duration) -> Vec<HwReport> {
    vec![
        rigaya(EncoderKind::NvEncC, "NVEncC", timeout),
        rigaya(EncoderKind::QsvEncC, "QSVEncC", timeout),
        rigaya(EncoderKind::VceEncC, "VCEEncC", timeout),
        vaapi(vaapi_device, timeout),
    ]
}

/// Is `codec` usable on the report's device? 10-bit HEVC needs "hevc10"
/// when the tool says so explicitly; otherwise plain HEVC is accepted.
pub fn supports(report: &HwReport, codec: &str) -> bool {
    report.usable && report.codecs.iter().any(|c| c == codec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rigaya_check_hw() {
        let nv = "#0: NVIDIA GeForce RTX 4070 (5888 cores, 2475 MHz)[PCIe4x16][560.94]\nAvailable Codec(s)\nH.264/AVC\nH.265/HEVC\nAV1\n";
        let (dev, codecs) = parse_rigaya(nv);
        assert_eq!(dev, "NVIDIA GeForce RTX 4070");
        assert_eq!(codecs, vec!["h264", "hevc", "av1"]);

        let qsv = "Success: QuickSyncVideo (hw encoder) available\nDevice #0: Intel Arc A380\nH.264/AVC\nHEVC 10bit\n";
        let (dev, codecs) = parse_rigaya(qsv);
        assert_eq!(dev, "Intel Arc A380");
        assert_eq!(codecs, vec!["h264", "hevc", "hevc10"]);

        let report = HwReport {
            kind: EncoderKind::QsvEncC,
            installed: true,
            usable: true,
            device: dev,
            codecs,
            note: String::new(),
        };
        assert_eq!(report.summary(), "Intel Arc A380: H.264, HEVC 10-bit");
        assert!(supports(&report, "hevc"));
        assert!(!supports(&report, "av1"));
        assert_eq!(
            parse_rigaya("Error: no NVIDIA device found").1,
            Vec::<String>::new()
        );
    }

    #[test]
    fn vainfo() {
        let out = "vainfo: VA-API version: 1.22 (libva 2.22.0)\nvainfo: Driver version: Mesa Gallium driver 24.2.0 for AMD Radeon RX 6800\nvainfo: Supported profile and entrypoints\n      VAProfileH264High               :\tVAEntrypointVLD\n      VAProfileH264High               :\tVAEntrypointEncSlice\n      VAProfileHEVCMain               :\tVAEntrypointEncSlice\n      VAProfileHEVCMain10             :\tVAEntrypointEncSlice\n      VAProfileAV1Profile0            :\tVAEntrypointVLD\n";
        let (driver, codecs) = parse_vainfo(out);
        assert!(driver.starts_with("Mesa Gallium"));
        assert_eq!(codecs, vec!["h264", "hevc", "hevc10"]);
        let intel = "      VAProfileAV1Profile0            :\tVAEntrypointEncSliceLP\n";
        assert_eq!(parse_vainfo(intel).1, vec!["av1"]);
    }

    #[test]
    fn missing_tools_report_clearly() {
        let r = vaapi("/dev/dri/does-not-exist", Duration::from_secs(2));
        assert!(!r.usable);
        assert!(r.note.contains("does not exist"));
        let n = rigaya(
            EncoderKind::NvEncC,
            "NVEncC-not-here",
            Duration::from_secs(2),
        );
        assert!(!n.installed);
        assert_eq!(n.summary(), "NVEncC-not-here is not installed");
    }
}
