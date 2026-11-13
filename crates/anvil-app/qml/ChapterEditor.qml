// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.anvil.app

// Chapters: keep the source's, leave them out, or edit a list (rename,
// retime, add at the preview position, generate, import, export).
Card {
    id: ed
    property var ctl
    // 0..1 position of the preview, for "Add here".
    property real position: 0.3
    signal importFile()
    signal exportFile()
    title: "Chapters"
    trailing: ctl.chaptersummary

    readonly property var rows: ctl.chapters.length ? ctl.chapters.split("\n").map(l => l.split("\t")) : []
    readonly property bool custom: ctl.chaptermode === "custom"
    property bool showAll: false
    readonly property int shown: showAll || rows.length <= 8 ? rows.length : 8

    RowLayout {
        Layout.fillWidth: true
        spacing: 8
        Select {
            Layout.preferredWidth: 220
            value: ed.ctl.chaptermode
            options: [["keep", "Keep the source's chapters"], ["custom", "Edit chapters"], ["none", "No chapters"]]
            onPicked: (v) => ed.ctl.chapmode(v)
        }
        Item { Layout.fillWidth: true }
        PrimaryButton { text: "Import…"; quiet: true; onClicked: ed.importFile() }
        PrimaryButton { text: "Export…"; quiet: true; enabled: ed.rows.length > 0; onClicked: ed.exportFile() }
    }

    Repeater {
        model: ed.rows.slice(0, ed.shown)
        RowLayout {
            id: crow
            required property var modelData
            required property int index
            Layout.fillWidth: true
            spacing: 8
            Text { text: (crow.index + 1) + "."; color: Tokens.dim; font.pixelSize: 12; Layout.preferredWidth: 22 }
            Field {
                Layout.preferredWidth: 110
                enabled: ed.custom
                committedText: crow.modelData[0]
                font.family: Tokens.mono
                onCommitted: (t) => ed.ctl.chaptime(crow.index, t)
            }
            Field {
                Layout.fillWidth: true
                enabled: ed.custom
                committedText: crow.modelData[1]
                onCommitted: (t) => ed.ctl.chaprename(crow.index, t)
            }
            IconButton { visible: ed.custom; text: "✕"; onClicked: ed.ctl.chapremove(crow.index) }
        }
    }
    PrimaryButton {
        visible: ed.rows.length > 8
        text: ed.showAll ? "Show fewer" : "Show all " + ed.rows.length
        quiet: true
        onClicked: ed.showAll = !ed.showAll
    }

    RowLayout {
        Layout.fillWidth: true
        visible: ed.custom
        spacing: 8
        PrimaryButton { text: "Add at preview position"; quiet: true; onClicked: ed.ctl.chapadd(ed.position) }
        Select {
            id: every
            Layout.preferredWidth: 120
            value: "5"
            options: [["5", "Every 5 min"], ["10", "Every 10 min"], ["15", "Every 15 min"], ["20", "Every 20 min"]]
            onPicked: (v) => value = v
        }
        PrimaryButton { text: "Generate"; quiet: true; onClicked: ed.ctl.chapevery(parseInt(every.value)) }
        PrimaryButton { text: "Renumber"; quiet: true; onClicked: ed.ctl.chaprenumber() }
    }
    Text {
        visible: !ed.custom && ed.ctl.chaptermode === "keep" && ed.rows.length > 0
        text: "Choose “Edit chapters” to rename, retime or add chapters."
        color: Tokens.dim
        font.pixelSize: 11
    }
}
