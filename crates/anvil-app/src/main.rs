// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Anvil desktop app entry point.

mod bridge;
mod prefs;
mod preset_editor;
mod queue_bridge;
mod tokens;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QQuickStyle, QString, QUrl};

extern "C" {
    /// Window icon and desktop file name (src/identity.cpp).
    fn anvil_set_identity(version: *const std::os::raw::c_char);
}

fn main() {
    anvil_core::paths::prepare_vapoursynth_env();
    // Presets and the queue from when Anvil was called Reelforge.
    let home = std::env::var("HOME").ok();
    anvil_core::paths::migrate_from_reelforge(home.as_deref());
    let mut app = QGuiApplication::new();
    let version = std::ffi::CString::new(env!("CARGO_PKG_VERSION")).expect("version");
    // SAFETY: called once, after the QGuiApplication exists; `version` is a
    // valid C string that outlives the call (Qt copies it).
    unsafe { anvil_set_identity(version.as_ptr()) };
    // A neutral, fully styleable base; the look comes from the Tokens singleton.
    QQuickStyle::set_style(&QString::from("Basic"));

    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/com/anvil/app/qml/Main.qml"));
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
