// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Anvil core engine.
//!
//! Everything that decides *how* a file is encoded lives here, with no Qt
//! dependency, so it can be unit tested and shared by the CLI and the GUI.
//!
//! Module map:
//! - [`model`]     source description: video, audio, subtitle tracks
//! - [`probe`]     turns `ffprobe` JSON into a [`model::SourceInfo`]
//! - [`hdr`]       HDR10 static metadata and encoder argument formatting
//! - [`audio`]     the default audio rule: native lossless first, then AC-3
//! - [`crop`]      crop values, mod-2 validation and auto-crop aggregation
//! - [`preset`]    layered TOML presets (built-in, user, per-job override)
//! - [`scriptgen`] VapourSynth filter profiles rendered to `.vpy`
//! - [`encoder`]   encoder command-line builders (x265, SVT-AV1, ...)
//! - [`command`]   a process invocation, with shell-quoted display
//! - [`pipeline`]  turns a source + preset into an ordered list of steps

pub mod audio;
pub mod chapters;
pub mod command;
pub mod crop;
pub mod disc;
pub mod dvd;
pub mod encoder;
pub mod error;
pub mod estimate;
pub mod hdr;
pub mod hdrmeta;
pub mod hw;
pub mod jobfile;
pub mod model;
pub mod ocr;
pub mod paths;
pub mod pipeline;
pub mod preset;
pub mod preview;
pub mod probe;
pub mod process;
pub mod progress;
pub mod queue;
pub mod runner;
pub mod schema;
pub mod scriptgen;
pub mod settings;
pub mod sizing;
pub mod subtitles;
pub mod tools;
pub mod trackmap;
pub mod verify;
pub mod vobsub;

pub use error::{Error, Result};
