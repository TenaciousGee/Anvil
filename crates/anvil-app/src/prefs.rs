// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! App preferences exposed to QML as the `Prefs` singleton, saved to
//! `~/.config/anvil/settings.toml` (shared with the CLI and the install
//! script; see `anvil_core::settings`).
//!
//! `forge`: Forge mode, which renames pages and buttons after the forge
//! (ore, alloys, the smelter, the hammer, tempering, the assay) and warms
//! the accent colour to ember. Off by default; it changes words only, never
//! what a job does.
//!
//! `hardware`: whether GPU encoders are used. Off means CPU encoding only:
//! GPU presets are hidden and the GPU checks are skipped.

use std::pin::Pin;

use anvil_core::settings::Settings;

#[cxx_qt::bridge]
pub mod qobject {
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(bool, forge)]
        #[qproperty(bool, hardware)]
        type Prefs = super::PrefsRust;

        /// Turn Forge mode on or off and remember the choice.
        #[qinvokable]
        fn setforge(self: Pin<&mut Prefs>, on: bool);

        /// Use GPU encoders or not (CPU only), and remember the choice.
        #[qinvokable]
        fn sethardware(self: Pin<&mut Prefs>, on: bool);
    }
}

pub struct PrefsRust {
    forge: bool,
    hardware: bool,
}

impl Default for PrefsRust {
    fn default() -> Self {
        let s = Settings::load();
        Self {
            forge: s.forge,
            hardware: s.hardware_encoders,
        }
    }
}

impl qobject::Prefs {
    fn save(&self) {
        let _ = Settings {
            forge: *self.forge(),
            hardware_encoders: *self.hardware(),
        }
        .save();
    }

    pub fn setforge(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().set_forge(on);
        self.save();
    }

    pub fn sethardware(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().set_hardware(on);
        self.save();
    }
}
