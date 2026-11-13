// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Normalise paths that arrive from drag and drop, file dialogs or typing.

/// Turn `file:///home/me/My%20Film%20(2026).mkv`, `~/rips/x.mkv` or a quoted
/// path into a plain filesystem path.
pub fn normalise_input_path(raw: &str, home: Option<&str>) -> String {
    let mut s = raw.trim();
    // Pasted paths are often quoted by file managers or shells.
    for q in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            s = &s[1..s.len() - 1];
        }
    }
    let decoded;
    let s = if let Some(rest) = s.strip_prefix("file://") {
        // file://host/path is rare; file:///path is the norm.
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        decoded = percent_decode(rest);
        decoded.as_str()
    } else {
        s
    };
    match (s.strip_prefix('~'), home) {
        (Some(rest), Some(h)) if rest.is_empty() || rest.starts_with('/') => format!("{h}{rest}"),
        _ => s.to_string(),
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Anvil was called Reelforge before November 2026. On first start, move
/// its config folder (user presets) and data folder (queue, thumbnails)
/// to the new names, and leave a link at each old path so anything that
/// stored a full path (thumbnails in the queue database) still resolves.
/// Returns the folders moved.
pub fn migrate_from_reelforge(home: Option<&str>) -> Vec<std::path::PathBuf> {
    let base = |var: &str, fallback: &str| {
        std::env::var_os(var)
            .map(std::path::PathBuf::from)
            .or_else(|| home.map(|h| std::path::Path::new(h).join(fallback)))
    };
    migrate_dirs(
        [
            base("XDG_CONFIG_HOME", ".config"),
            base("XDG_DATA_HOME", ".local/share"),
        ]
        .into_iter()
        .flatten(),
    )
}

/// In each base folder, move `reelforge` to `anvil` (unless `anvil` exists)
/// and link the old name to the new one.
fn migrate_dirs(bases: impl IntoIterator<Item = std::path::PathBuf>) -> Vec<std::path::PathBuf> {
    let mut moved = Vec::new();
    for dir in bases {
        let (old, new) = (dir.join("reelforge"), dir.join("anvil"));
        let old_is_real = old.is_dir() && !old.is_symlink();
        if old_is_real && !new.exists() && std::fs::rename(&old, &new).is_ok() {
            #[cfg(unix)]
            let _ = std::os::unix::fs::symlink(&new, &old);
            moved.push(new);
        }
    }
    moved
}

/// Anvil's own VapourSynth plugin folder, filled from the plugins' official
/// PyPI builds by `packaging/scripts/install-vs-plugins.py`:
/// `~/.local/share/anvil/vs/vapoursynth/plugins`.
pub fn vapoursynth_plugin_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
        })
        .map(|d| d.join("anvil/vs/vapoursynth/plugins"))
}

/// The variable VapourSynth R80 and later read for an extra plugin folder
/// (they no longer read vapoursynth.conf or a user plugin folder).
pub const VS_EXTRA_PLUGIN_ENV: &str = "VAPOURSYNTH_EXTRA_PLUGIN_PATH";

/// Point VapourSynth at Anvil's plugin folder for this process and every
/// tool it starts (vspipe, the preview, the plugin check). Leaves a value
/// the user set alone. Call at the very start of `main`, before threads.
pub fn prepare_vapoursynth_env() {
    if std::env::var_os(VS_EXTRA_PLUGIN_ENV).is_some_and(|v| !v.is_empty()) {
        return;
    }
    if let Some(dir) = vapoursynth_plugin_dir().filter(|d| d.is_dir()) {
        // Single-threaded at this point (start of main).
        std::env::set_var(VS_EXTRA_PLUGIN_ENV, dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_dropped_urls() {
        assert_eq!(
            normalise_input_path("file:///mnt/rips/Film%20Name%20(2026)/title_t00.mkv", None),
            "/mnt/rips/Film Name (2026)/title_t00.mkv"
        );
        assert_eq!(
            normalise_input_path("file:///m/Am%C3%A9lie.mkv", None),
            "/m/Amélie.mkv"
        );
        assert_eq!(normalise_input_path("file://localhost/a%2", None), "/a%2");
    }

    #[test]
    fn typed_paths() {
        assert_eq!(normalise_input_path("  '/a b/c.mkv' ", None), "/a b/c.mkv");
        assert_eq!(
            normalise_input_path("~/rips/x.mkv", Some("/home/me")),
            "/home/me/rips/x.mkv"
        );
        assert_eq!(
            normalise_input_path("~other/x.mkv", Some("/home/me")),
            "~other/x.mkv"
        );
        // A literal % in a typed path is left alone.
        assert_eq!(
            normalise_input_path("/r/100%25 fun.mkv", None),
            "/r/100%25 fun.mkv"
        );
    }

    #[test]
    fn moves_reelforge_folders_once() {
        let base = std::env::temp_dir().join(format!("anvil-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("reelforge/presets")).unwrap();
        std::fs::write(base.join("reelforge/presets/mine.toml"), "x").unwrap();
        assert_eq!(migrate_dirs([base.clone()]), vec![base.join("anvil")]);
        assert!(base.join("anvil/presets/mine.toml").is_file());
        // The old path still leads to the files.
        assert!(base.join("reelforge/presets/mine.toml").is_file());
        assert!(base.join("reelforge").is_symlink());
        // Second start: nothing to do.
        assert!(migrate_dirs([base.clone()]).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }
}
