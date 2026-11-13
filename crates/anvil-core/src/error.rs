// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
use std::path::PathBuf;

/// Errors raised by the core engine.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not parse ffprobe output: {0}")]
    ProbeParse(#[from] serde_json::Error),

    #[error("source has no video stream")]
    NoVideo,

    #[error("invalid value for {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    #[error("preset error: {0}")]
    Preset(String),

    #[error("could not parse preset TOML: {0}")]
    PresetToml(#[from] toml::de::Error),

    #[error("unknown preset '{0}'")]
    UnknownPreset(String),

    #[error("{encoder} cannot be used here: {reason}")]
    Unsupported { encoder: String, reason: String },

    #[error("{0} is not installed (or not on PATH)")]
    ToolMissing(String),

    #[error("{tool}: {message}")]
    Tool { tool: String, message: String },

    #[error("step '{step}' failed: {message}\n(full log: {})", log.display())]
    StepFailed {
        step: String,
        message: String,
        log: PathBuf,
    },

    #[error("cancelled")]
    Cancelled,

    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;
