// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

/// Every QML file in the module. A file missing here fails at runtime with
/// "X is not a type", so keep this list in step with the qml/ folder.
const QML_FILES: &[&str] = &[
    "qml/Main.qml",
    // Shared components
    "qml/Banner.qml",
    "qml/Card.qml",
    "qml/Chip.qml",
    "qml/IconButton.qml",
    "qml/NavButton.qml",
    "qml/PageHeader.qml",
    "qml/PreviewPane.qml",
    "qml/PrimaryButton.qml",
    "qml/Spinner.qml",
    "qml/Select.qml",
    "qml/Field.qml",
    "qml/Toggle.qml",
    "qml/Caption.qml",
    "qml/AudioEditor.qml",
    "qml/SubtitleEditor.qml",
    "qml/ChapterEditor.qml",
    // Pages
    "qml/EncodePage.qml",
    "qml/QueuePage.qml",
    "qml/PresetsPage.qml",
    "qml/FiltersPage.qml",
    "qml/ToolsPage.qml",
];

fn main() {
    // Fail the build, not the app, if a QML file is added but not listed.
    let listed: std::collections::BTreeSet<&str> = QML_FILES.iter().copied().collect();
    for entry in std::fs::read_dir("qml").expect("qml folder").flatten() {
        let path = format!("qml/{}", entry.file_name().to_string_lossy());
        if path.ends_with(".qml") && !listed.contains(path.as_str()) {
            panic!("{path} exists but is not listed in QML_FILES in build.rs");
        }
    }
    println!("cargo::rerun-if-changed=qml");
    // Editing a file in place does not change the folder's time, so list
    // each file too.
    for f in QML_FILES {
        println!("cargo::rerun-if-changed={f}");
    }

    CxxQtBuilder::new_qml_module(
        QmlModule::new("com.anvil.app").qml_files(QML_FILES.iter().map(|f| QmlFile::from(*f))),
    )
    .files([
        "src/bridge.rs",
        "src/queue_bridge.rs",
        "src/tokens.rs",
        "src/prefs.rs",
        "src/preset_editor.rs",
    ])
    .qrc_resources(["assets/anvil.png"])
    .cpp_file("src/identity.cpp")
    .qt_module("Quick")
    .qt_module("QuickControls2")
    .build();
}
