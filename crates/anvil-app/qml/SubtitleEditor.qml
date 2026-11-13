// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.anvil.app

// Output subtitle tracks: order, flags, language and title, files to add,
// and forced-only tracks picked out of Blu-ray subtitles.
Card {
    id: ed
    property var ctl
    signal addFile()
    title: "Subtitles"
    trailing: rows.length === 0 ? "None kept" : rows.length + (rows.length === 1 ? " track" : " tracks")

    property int open: -1
    readonly property var rows: ctl.subs.length ? ctl.subs.split("\n").map(l => l.split("\t")) : []
    readonly property var sources: ctl.subsources.length ? ctl.subsources.split("\n").map(l => l.split("\t")) : []

    Repeater {
        id: rep
        model: ed.rows
        Rectangle {
            id: row
            required property var modelData
            required property int index
            readonly property bool isOpen: ed.open === index
            function send(lang, title, def, forced, only, ocr) {
                ed.ctl.subedit(row.index, lang, title, def, forced, only, ocr === undefined ? row.ocr : ocr)
            }
            readonly property string lang: modelData[1]
            readonly property string name: modelData[2]
            readonly property bool def: modelData[3] === "1"
            readonly property bool forced: modelData[4] === "1"
            readonly property bool only: modelData[5] === "1"
            readonly property bool ocr: modelData[8] === "1"

            Layout.fillWidth: true
            implicitHeight: col.implicitHeight + 16
            radius: 6
            color: isOpen ? Tokens.tint : Tokens.raised
            border.color: isOpen ? Tokens.accent : "transparent"

            ColumnLayout {
                id: col
                anchors.fill: parent
                anchors.margins: 8
                anchors.leftMargin: 12
                spacing: 10
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8
                    Text { text: (row.index + 1) + "."; color: Tokens.dim; font.pixelSize: Tokens.body }
                    Text {
                        Layout.fillWidth: true
                        text: row.modelData[0]
                        color: Tokens.text; font.pixelSize: Tokens.body; elide: Text.ElideRight
                        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: ed.open = row.isOpen ? -1 : row.index }
                    }
                    Chip { text: "default"; visible: row.def; tone: Tokens.good }
                    Chip { text: "forced"; visible: row.forced; tone: Tokens.warn }
                    Chip { text: "OCR"; visible: row.ocr; tone: Tokens.accent }
                    IconButton { text: "▲"; enabled: row.index > 0; onClicked: { ed.ctl.submove(row.index, row.index - 1); ed.open = -1 } }
                    IconButton { text: "▼"; enabled: row.index < rep.count - 1; onClicked: { ed.ctl.submove(row.index, row.index + 1); ed.open = -1 } }
                    IconButton { text: row.isOpen ? "✓" : "✎"; onClicked: ed.open = row.isOpen ? -1 : row.index }
                }
                GridLayout {
                    visible: row.isOpen
                    Layout.fillWidth: true
                    columns: 3
                    columnSpacing: 10
                    rowSpacing: 4
                    Caption { text: "Language" }
                    Caption { text: "Title" }
                    Item { Layout.preferredWidth: 1 }
                    Field {
                        Layout.preferredWidth: 90
                        committedText: row.lang
                        placeholderText: "eng"
                        maximumLength: 3
                        onCommitted: (t) => row.send(t, row.name, row.def, row.forced, row.only)
                    }
                    Field {
                        Layout.fillWidth: true
                        committedText: row.name
                        placeholderText: "Track name shown by players"
                        onCommitted: (t) => row.send(row.lang, t, row.def, row.forced, row.only)
                    }
                    PrimaryButton {
                        text: "Remove"; quiet: true; danger: true
                        onClicked: { ed.open = -1; ed.ctl.subremove(row.index) }
                    }
                    Flow {
                        Layout.columnSpan: 3
                        Layout.fillWidth: true
                        spacing: 16
                        Toggle { text: "Default track"; checked: row.def; onToggled: row.send(row.lang, row.name, checked, row.forced, row.only) }
                        Toggle { text: "Forced"; checked: row.forced; enabled: !row.only; onToggled: row.send(row.lang, row.name, row.def, checked, row.only) }
                        Toggle {
                            visible: row.modelData[6] === "1"
                            text: "Forced captions only (for foreign-language lines)"
                            checked: row.only
                            onToggled: row.send(row.lang, row.name, row.def, checked, checked)
                        }
                        Toggle {
                            visible: row.modelData[7] === "1"
                            text: "Convert to text (OCR with Tesseract)"
                            checked: row.ocr
                            onToggled: row.send(row.lang, row.name, row.def, row.forced, row.only, checked)
                        }
                    }
                }
            }
        }
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: 8
        Select {
            id: addSel
            Layout.fillWidth: true
            visible: ed.sources.length > 0
            options: ed.sources.map(s => [s[0], "Add " + s[1]])
            value: options.length ? options[0][0] : ""
            onPicked: (v) => value = v
        }
        PrimaryButton { visible: ed.sources.length > 0; text: "Add track"; quiet: true; onClicked: ed.ctl.subadd(parseInt(addSel.value)) }
        Item { Layout.fillWidth: true; visible: ed.sources.length === 0 }
        PrimaryButton { text: "Add file…"; quiet: true; onClicked: ed.addFile() }
    }
}
