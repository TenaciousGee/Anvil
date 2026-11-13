// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.anvil.app

// Which encoding tools are installed, with versions.
ScrollView {
    id: page
    property var ctl
    contentWidth: availableWidth
    clip: true

    function lines(s) { return s.length ? s.split("\n") : [] }
    // In CPU-only mode the GPU encoders are not listed at all.
    readonly property var gpuTools: ["NVEncC", "QSVEncC", "VCEEncC"]
    readonly property var rows: lines(ctl.tools).map(l => l.split("\t"))
        .filter(r => Prefs.hardware || gpuTools.indexOf(r[0]) < 0)
    readonly property var hw: lines(ctl.hardware).map(l => l.split("\t"))
    readonly property int missing: rows.filter(r => r[1] === "missing").length

    ColumnLayout {
        width: page.availableWidth
        spacing: 0

        ColumnLayout {
            Layout.fillWidth: true
            Layout.margins: 24
            spacing: Tokens.gap

            RowLayout {
                Layout.fillWidth: true
                PageHeader {
                    Layout.fillWidth: true
                    title: Prefs.forge ? "Tongs" : "Tools"
                    subtitle: page.ctl.scanning ? "Checking…" : (page.missing === 0 ? "Everything required is installed." : page.missing + " required " + (page.missing === 1 ? "tool is" : "tools are") + " missing.")
                }
                Spinner { running: page.ctl.scanning }
                PrimaryButton { text: "Check again"; quiet: true; enabled: !page.ctl.scanning; onClicked: page.ctl.scan() }
            }

            Card {
                Layout.fillWidth: true
                title: "Hardware encoders"
                trailing: Prefs.hardware ? page.hw.filter(r => r[1] === "1").length + " usable" : "Off: CPU encoding only"
                Toggle {
                    text: "Use GPU encoders (NVIDIA, Intel, AMD, VAAPI)"
                    checked: Prefs.hardware
                    onToggled: { Prefs.sethardware(checked); page.ctl.reloadpresets(""); page.ctl.scan() }
                }
                Text {
                    Layout.fillWidth: true
                    visible: !Prefs.hardware
                    wrapMode: Text.WordWrap
                    text: "Anvil uses the CPU encoders only and hides the GPU presets. Turn this on after installing your graphics card's encoder (./install-cachyos.sh --gpu)."
                    color: Tokens.dim
                    font.pixelSize: 12
                }
                Repeater {
                    model: Prefs.hardware ? page.hw : []
                    RowLayout {
                        required property var modelData
                        Layout.fillWidth: true
                        spacing: 10
                        Rectangle { width: 8; height: 8; radius: 4; color: modelData[1] === "1" ? Tokens.good : Tokens.dim }
                        Text { text: modelData[0]; color: Tokens.text; font.pixelSize: Tokens.body; Layout.preferredWidth: 130 }
                        Text { text: modelData[2]; color: modelData[1] === "1" ? Tokens.text : Tokens.dim; font.pixelSize: 12; Layout.fillWidth: true; wrapMode: Text.WordWrap }
                    }
                }
                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    color: Tokens.dim
                    font.pixelSize: 12
                    text: "The Fast presets use these: NVIDIA (NVEncC), Intel Quick Sync (QSVEncC), AMD (VCEEncC) or VAAPI through FFmpeg, which needs no extra encoder. Each GPU encodes one job at a time beside the CPU queue. To check them end to end, Dolby Vision included, run: anvil selftest"
                }
            }

            Card {
                Layout.fillWidth: true
                title: "Appearance"
                Toggle {
                    text: "Forge mode: metalworking names for pages, buttons and job states"
                    checked: Prefs.forge
                    onToggled: Prefs.setforge(checked)
                }
                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: Prefs.forge
                        ? "Your rip is the raw ore and Anvil the forge. Jobs heat in the Smelter (the queue), go under the hammer (the encode), are tempered (muxed) and assayed (checked). Presets are Alloys, filters are Flux and these tools are the Tongs. Only the words change; every job runs exactly the same."
                        : "Off: plain names. Turn it on for a forge to match the name: ore, the smelter, the hammer, tempering and alloys. Only the words change."
                    color: Tokens.dim
                    font.pixelSize: 12
                }
            }

            Card {
                Layout.fillWidth: true
                visible: page.missing > 0
                color: Tokens.caution
                border.color: "#59F2B35B"
                title: "Install the missing tools"
                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: "On CachyOS, run this from the Anvil folder, then press Check again:"
                    color: Tokens.text
                    font.pixelSize: Tokens.body
                }
                TextEdit {
                    Layout.fillWidth: true
                    readOnly: true
                    selectByMouse: true
                    text: "./install-cachyos.sh --with-tools --with-aur"
                    color: Tokens.accent
                    font.family: Tokens.mono
                    font.pixelSize: 13
                }
            }

            Card {
                Layout.fillWidth: true
                title: "Encoding tools"
                trailing: page.rows.filter(r => r[1] === "ok").length + " of " + page.rows.length + " found"
                Repeater {
                    model: page.rows
                    Rectangle {
                        id: toolRow
                        required property var modelData
                        Layout.fillWidth: true
                        implicitHeight: 48
                        radius: 6
                        color: Tokens.raised
                        readonly property color dot: modelData[1] === "ok" ? Tokens.good : (modelData[1] === "missing" ? "#FF6B6B" : Tokens.border)
                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: 14
                            anchors.rightMargin: 14
                            spacing: 12
                            Rectangle { width: 10; height: 10; radius: 5; color: toolRow.dot }
                            Text { text: toolRow.modelData[0]; color: Tokens.text; font.pixelSize: Tokens.body; font.weight: Font.DemiBold; Layout.preferredWidth: 130 }
                            Text { text: toolRow.modelData[3]; color: Tokens.dim; font.pixelSize: 12; Layout.preferredWidth: 220; elide: Text.ElideRight }
                            Text {
                                Layout.fillWidth: true
                                text: toolRow.modelData[1] === "missing" ? "Required, not installed" : toolRow.modelData[2]
                                color: toolRow.modelData[1] === "missing" ? "#FF8A8A" : (toolRow.modelData[1] === "ok" ? Tokens.text : Tokens.dim)
                                font.pixelSize: 12
                                elide: Text.ElideRight
                                horizontalAlignment: Text.AlignRight
                            }
                        }
                    }
                }
            }

            Card {
                Layout.fillWidth: true
                title: "About Anvil " + Qt.application.version
                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    textFormat: Text.StyledText
                    linkColor: Tokens.accent
                    onLinkActivated: link => Qt.openUrlExternally(link)
                    text: "Anvil is free software: you can redistribute it and/or modify it under the terms of the "
                        + "<a href=\"https://www.gnu.org/licenses/gpl-3.0.html\">GNU General Public License</a>, version 3 or (at your option) any later version. "
                        + "It comes with ABSOLUTELY NO WARRANTY."
                    color: Tokens.text
                    font.pixelSize: 12
                }
                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: "Anvil runs the tools listed above as separate programs, each under its own licence (x264, x265, FFmpeg, MKVToolNix, VapourSynth and others). "
                        + "The AppImage and Flatpak include the Qt 6 libraries under the GNU LGPL version 3; you may replace them with your own build of Qt. "
                        + "THIRD-PARTY.md, installed with Anvil, lists every component, its licence and where to get its source."
                    color: Tokens.dim
                    font.pixelSize: 12
                }
            }
        }
    }
}
