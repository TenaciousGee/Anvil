// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.anvil.app

// Output audio tracks: order, and per track the format, bitrate, channels,
// delay, loudness and title. Rows open for editing one at a time.
Card {
    id: ed
    property var ctl
    title: "Audio"
    trailing: "Lossless first, then AC-3 · click a track to change it"

    // Which row is open; kept here so a re-plan does not close it.
    property int open: -1
    readonly property var rows: ctl.audio.length ? ctl.audio.split("\n").map(l => l.split("\t")) : []
    readonly property var sources: ctl.audiosources.length ? ctl.audiosources.split("\n").map(l => l.split("\t")) : []

    readonly property var bitrates: ({
        ac3: [192, 224, 256, 384, 448, 640],
        eac3: [256, 384, 448, 640, 768, 1024, 1536],
        aac: [128, 160, 192, 256, 320, 384, 448, 512],
        opus: [96, 128, 160, 192, 256, 320, 384, 448, 512]
    })
    function layout(n) { return ({ 1: "Mono", 2: "Stereo", 6: "5.1", 8: "7.1" })[n] || (n + " channels") }

    Repeater {
        id: rep
        model: ed.rows
        Rectangle {
            id: row
            required property var modelData
            required property int index
            readonly property bool isOpen: ed.open === index
            readonly property string target: modelData[1]
            readonly property bool reencodes: target !== "copy" && target !== "dts_core"
            readonly property int maxch: Math.min(parseInt(modelData[8]), ["ac3", "eac3"].indexOf(target) >= 0 ? 6 : 8)
            function send(t, b, c, d, n, title) {
                ed.ctl.audioedit(row.index, t, b, c, d, n, title)
            }

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
                    Chip { text: "default"; visible: row.modelData[10] === "1"; tone: Tokens.good }
                    IconButton { text: "▲"; enabled: row.index > 0; onClicked: { ed.ctl.reorder(row.index, row.index - 1); ed.open = -1 } }
                    IconButton { text: "▼"; enabled: row.index < rep.count - 1; onClicked: { ed.ctl.reorder(row.index, row.index + 1); ed.open = -1 } }
                    IconButton { text: row.isOpen ? "✓" : "✎"; onClicked: ed.open = row.isOpen ? -1 : row.index }
                }

                GridLayout {
                    visible: row.isOpen
                    Layout.fillWidth: true
                    columns: 3
                    columnSpacing: 10
                    rowSpacing: 4

                    Caption { text: "Format (from " + row.modelData[6] + ")" }
                    Caption { text: "Bitrate"; visible: row.reencodes && row.target !== "flac" }
                    Caption { text: "Channels"; visible: row.reencodes }
                    Item { visible: !(row.reencodes && row.target !== "flac"); Layout.preferredWidth: 1 }
                    Item { visible: !row.reencodes; Layout.preferredWidth: 1 }

                    Select {
                        Layout.fillWidth: true
                        Layout.columnSpan: row.reencodes ? 1 : 3
                        value: row.target
                        options: [["copy", "Keep as it is (no re-encode)"], ["ac3", "Dolby Digital (AC-3)"], ["eac3", "Dolby Digital Plus (E-AC-3)"],
                                  ["aac", "AAC"], ["opus", "Opus"], ["flac", "FLAC (lossless)"]]
                                 .concat(row.modelData[9] === "1" ? [["dts_core", "DTS core (no re-encode)"]] : [])
                        // A new format starts from its own default bitrate.
                        onPicked: (v) => row.send(v, 0, parseInt(row.modelData[3]), parseInt(row.modelData[4]), row.modelData[5] === "1", row.modelData[7])
                    }
                    Select {
                        visible: row.reencodes && row.target !== "flac"
                        Layout.preferredWidth: 120
                        value: row.modelData[2]
                        options: (ed.bitrates[row.target] || []).map(b => [String(b), b + " kb/s"])
                        onPicked: (v) => row.send(row.target, parseInt(v), parseInt(row.modelData[3]), parseInt(row.modelData[4]), row.modelData[5] === "1", row.modelData[7])
                    }
                    Item { visible: row.reencodes && row.target === "flac"; Layout.preferredWidth: 1 }
                    Select {
                        visible: row.reencodes
                        Layout.preferredWidth: 120
                        value: row.modelData[3]
                        options: [1, 2, 6, 8].filter(n => n <= row.maxch).map(n => [String(n), ed.layout(n)])
                        onPicked: (v) => row.send(row.target, parseInt(row.modelData[2]), parseInt(v), parseInt(row.modelData[4]), row.modelData[5] === "1", row.modelData[7])
                    }

                    Caption { text: "Title" }
                    Caption { text: "Delay (ms)" }
                    Item { Layout.preferredWidth: 1 }
                    Field {
                        Layout.fillWidth: true
                        committedText: row.modelData[7]
                        placeholderText: "Track name shown by players"
                        onCommitted: (t) => row.send(row.target, parseInt(row.modelData[2]), parseInt(row.modelData[3]), parseInt(row.modelData[4]), row.modelData[5] === "1", t)
                    }
                    Field {
                        Layout.preferredWidth: 120
                        committedText: row.modelData[4]
                        validator: IntValidator { bottom: -10000; top: 10000 }
                        onCommitted: (t) => row.send(row.target, parseInt(row.modelData[2]), parseInt(row.modelData[3]), parseInt(t) || 0, row.modelData[5] === "1", row.modelData[7])
                    }
                    PrimaryButton {
                        text: "Remove"; quiet: true; danger: true
                        Layout.preferredWidth: 120
                        onClicked: { ed.open = -1; ed.ctl.audioremove(row.index) }
                    }

                    Toggle {
                        Layout.columnSpan: 3
                        visible: row.reencodes
                        text: "Normalise loudness to −23 LUFS (one fixed gain, mix untouched)"
                        checked: row.modelData[5] === "1"
                        onToggled: row.send(row.target, parseInt(row.modelData[2]), parseInt(row.modelData[3]), parseInt(row.modelData[4]), checked, row.modelData[7])
                    }
                }
            }
        }
    }
    Text { visible: rep.count === 0; text: "No audio tracks"; color: Tokens.dim; font.pixelSize: 12 }

    RowLayout {
        Layout.fillWidth: true
        visible: ed.sources.length > 0
        spacing: 8
        Select {
            id: addSel
            Layout.fillWidth: true
            options: ed.sources.map(s => [s[0], "Add " + s[1]])
            value: options.length ? options[0][0] : ""
            onPicked: (v) => value = v
        }
        PrimaryButton { text: "Add track"; quiet: true; onClicked: ed.ctl.audioadd(parseInt(addSel.value)) }
    }
}
