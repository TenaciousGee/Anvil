// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Detect the external tools Anvil drives, with their versions.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::process::run_with_timeout;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ToolSpec {
    pub name: &'static str,
    pub program: &'static str,
    pub version_args: &'static [&'static str],
    pub required: bool,
    pub purpose: &'static str,
}

/// Every tool the planner can call, in the order shown on the Tools page.
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "FFprobe",
        program: "ffprobe",
        version_args: &["-version"],
        required: true,
        purpose: "Reads files and detects HDR",
    },
    ToolSpec {
        name: "FFmpeg",
        program: "ffmpeg",
        version_args: &["-version"],
        required: true,
        purpose: "Audio encoding, auto-crop",
    },
    ToolSpec {
        name: "VapourSynth",
        program: "vspipe",
        version_args: &["--version"],
        required: true,
        purpose: "Filtering and frame serving",
    },
    ToolSpec {
        name: "x265",
        program: "x265",
        version_args: &["--version"],
        required: true,
        purpose: "Default HEVC encoder (CPU)",
    },
    ToolSpec {
        name: "MKVToolNix",
        program: "mkvmerge",
        version_args: &["--version"],
        required: true,
        purpose: "Muxing to MKV",
    },
    ToolSpec {
        name: "mkvextract",
        program: "mkvextract",
        version_args: &["--version"],
        required: true,
        purpose: "VFR timecodes",
    },
    ToolSpec {
        name: "MediaInfo",
        program: "mediainfo",
        version_args: &["--Version"],
        required: false,
        purpose: "Detailed checks",
    },
    ToolSpec {
        name: "Tesseract",
        program: "tesseract",
        version_args: &["--version"],
        required: false,
        purpose: "Subtitle OCR to text",
    },
    ToolSpec {
        name: "dovi_tool",
        program: "dovi_tool",
        version_args: &["--version"],
        required: false,
        purpose: "Dolby Vision RPU",
    },
    ToolSpec {
        name: "hdr10plus_tool",
        program: "hdr10plus_tool",
        version_args: &["--version"],
        required: false,
        purpose: "HDR10+ metadata",
    },
    ToolSpec {
        name: "SVT-AV1",
        program: "SvtAv1EncApp",
        version_args: &["--version"],
        required: false,
        purpose: "AV1 encoder (CPU)",
    },
    ToolSpec {
        name: "x264",
        program: "x264",
        version_args: &["--version"],
        required: false,
        purpose: "AVC encoder (CPU)",
    },
    ToolSpec {
        name: "rav1e",
        program: "rav1e",
        version_args: &["--version"],
        required: false,
        purpose: "AV1 encoder (CPU)",
    },
    ToolSpec {
        name: "NVEncC",
        program: "NVEncC",
        version_args: &["--version"],
        required: false,
        purpose: "NVIDIA hardware encoder",
    },
    ToolSpec {
        name: "QSVEncC",
        program: "QSVEncC",
        version_args: &["--version"],
        required: false,
        purpose: "Intel hardware encoder",
    },
    ToolSpec {
        name: "VCEEncC",
        program: "VCEEncC",
        version_args: &["--version"],
        required: false,
        purpose: "AMD hardware encoder",
    },
];

#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    pub name: &'static str,
    pub program: &'static str,
    pub required: bool,
    pub purpose: &'static str,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
}

impl ToolStatus {
    pub fn found(&self) -> bool {
        self.path.is_some()
    }
}

/// Look a program up on `PATH`.
/// Other names a tool is installed under. rigaya's encoders are
/// `NVEncC` on Windows but `nvencc`, `qsvencc` and `vceencc` in the Linux
/// packages (AUR, .deb and .rpm).
fn aliases(program: &str) -> &'static [&'static str] {
    match program {
        "NVEncC" => &["nvencc", "NVEncC64"],
        "QSVEncC" => &["qsvencc", "QSVEncC64"],
        "VCEEncC" => &["vceencc", "VCEEncC64"],
        _ => &[],
    }
}

/// Find a program on PATH, by its own name or a known alias.
pub fn which(program: &str) -> Option<PathBuf> {
    which_in(program, &std::env::var_os("PATH")?)
}

fn which_in(program: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    std::iter::once(program)
        .chain(aliases(program).iter().copied())
        .find_map(|name| {
            std::env::split_paths(path)
                .map(|d| d.join(name))
                .find(|p| is_executable(p))
        })
}

/// How to install a program that is missing, when there is more to say
/// than its name.
pub fn install_hint(program: &str) -> Option<&'static str> {
    Some(match program.to_ascii_lowercase().as_str() {
        "nvencc" | "nvencc64" => "install the AUR package nvenc (it builds with CUDA: sudo pacman -S cuda, then PATH=/opt/cuda/bin:$PATH paru -S nvenc), or pick a CPU preset",
        "qsvencc" | "qsvencc64" => "install the AUR package qsvenc (paru -S qsvenc; needs an Intel GPU), or pick a CPU preset",
        "vceencc" | "vceencc64" => "install the AUR package vceenc (paru -S vceenc; needs an AMD GPU with AMF), or pick a CPU preset",
        "vspipe" => "install vapoursynth (sudo pacman -S vapoursynth)",
        _ => return None,
    })
}

/// The name to run a program by: the installed alias when only that is
/// on PATH (`nvencc` for NVEncC), otherwise the name itself.
pub fn program(name: &str) -> String {
    match which(name) {
        Some(p) => p
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| name.to_string()),
        None => name.to_string(),
    }
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Pick a readable version string from a tool's output.
pub fn version_line(stdout: &str, stderr: &str) -> Option<String> {
    stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|l| !l.is_empty() && l.chars().any(|c| c.is_ascii_digit()))
        .map(|l| {
            // "x265 [info]: HEVC encoder version 3.6" -> "HEVC encoder version 3.6"
            let l = l.split_once("]: ").map(|(_, r)| r).unwrap_or(l);
            let l = l.split(" Copyright").next().unwrap_or(l);
            l.chars().take(80).collect()
        })
}

/// Detect every tool in parallel; each check is capped at `timeout`.
pub fn detect_all(timeout: Duration) -> Vec<ToolStatus> {
    let handles: Vec<_> = TOOLS
        .iter()
        .map(|spec| {
            let spec = *spec;
            std::thread::spawn(move || detect(&spec, timeout))
        })
        .collect();
    handles.into_iter().filter_map(|h| h.join().ok()).collect()
}

pub fn detect(spec: &ToolSpec, timeout: Duration) -> ToolStatus {
    let path = which(spec.program);
    let version = path.as_ref().and_then(|p| {
        let args: Vec<String> = spec.version_args.iter().map(|s| s.to_string()).collect();
        run_with_timeout(&p.to_string_lossy(), &args, timeout)
            .ok()
            .and_then(|o| version_line(&o.stdout, &o.stderr))
    });
    ToolStatus {
        name: spec.name,
        program: spec.program,
        required: spec.required,
        purpose: spec.purpose,
        path,
        version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_lines() {
        assert_eq!(
            version_line(
                "",
                "x265 [info]: HEVC encoder version 3.6\nx265 [info]: build info"
            ),
            Some("HEVC encoder version 3.6".into())
        );
        assert_eq!(
            version_line(
                "ffprobe version 7.1 Copyright (c) 2007-2024 the FFmpeg developers\n",
                ""
            ),
            Some("ffprobe version 7.1".into())
        );
        assert_eq!(version_line("\n\n", ""), None);
    }

    #[test]
    fn detects_a_present_and_a_missing_tool() {
        let sh = ToolSpec {
            name: "sh",
            program: "sh",
            version_args: &["-c", "echo sh 1.0"],
            required: false,
            purpose: "",
        };
        assert!(detect(&sh, Duration::from_secs(5)).found());
        let nope = ToolSpec {
            name: "x",
            program: "no-such-tool-xyz",
            version_args: &[],
            required: false,
            purpose: "",
        };
        let s = detect(&nope, Duration::from_secs(1));
        assert!(!s.found() && s.version.is_none());
    }

    #[test]
    fn rigaya_lowercase_names() {
        let dir = std::env::temp_dir().join(format!("anvil-alias-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("vceencc");
        std::fs::write(&exe, "#!/bin/sh\necho VCEEncC 8.40\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Searched in a PATH of our own: changing the real one would race
        // with other tests starting programs.
        let path = std::env::join_paths([dir.clone()]).unwrap();
        assert_eq!(which_in("VCEEncC", &path), Some(exe));
        assert_eq!(which_in("NVEncC", &path), None);
        assert_eq!(program("NVEncC-missing"), "NVEncC-missing");
        std::fs::remove_dir_all(&dir).ok();
    }
}
