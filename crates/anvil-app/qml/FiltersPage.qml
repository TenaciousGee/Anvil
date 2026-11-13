// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.anvil.app

// The open job's filter chain: blocks with simple settings, the plugins
// they need, and the exact VapourSynth script (generated, or edited by hand).
ScrollView {
    id: page
    property var ctl
    signal openEncode()
    contentWidth: availableWidth
    clip: true

    function lines(s) { return s.length ? s.split("\n") : [] }
    function t(plain, forge) { return Prefs.forge ? forge : plain }
    readonly property var rows: lines(ctl.chain).map(l => l.split("\t"))
    readonly property var catalogue: lines(ctl.catalogue).map(l => l.split("\t"))
    readonly property var plugins: lines(ctl.plugins).map(l => l.split("\t"))
    readonly property int missing: plugins.filter(p => p[1] === "0").length
    readonly property bool custom: ctl.scriptmode === "custom"
    property bool editing: false
    readonly property var strengths: [["light", "Light"], ["medium", "Medium"], ["strong", "Strong"]]
    readonly property var qtgmcPresets: ["Placebo", "Very Slow", "Slower", "Slow", "Medium", "Fast", "Faster", "Very Fast", "Super Fast", "Ultra Fast", "Draft"]

    ColumnLayout {
        width: page.availableWidth
        spacing: 0

        ColumnLayout {
            Layout.fillWidth: true
            Layout.margins: 24
            spacing: Tokens.gap

            PageHeader {
                Layout.fillWidth: true
                title: Prefs.forge ? "Flux" : "Filters"
                subtitle: page.t("The VapourSynth chain for the open file. The same script drives the preview and the encode, so what you see is what you get.",
                                 "Flux cleans the metal before it is worked: the VapourSynth chain for the open ore. The same script drives the preview and the encode.")
            }

            Card {
                Layout.fillWidth: true
                visible: !page.ctl.loaded
                Text { text: "Open a file to edit its filters"; color: Tokens.text; font.pixelSize: Tokens.heading; font.weight: Font.DemiBold }
                Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; text: "Interlaced DVDs get deinterlacing added automatically, and auto-crop adds a crop step. With no filters, Anvil decodes straight from FFmpeg, which is fastest."; color: Tokens.dim; font.pixelSize: Tokens.body }
                PrimaryButton { text: page.t("Go to Encode", "Go to the Forge"); onClicked: page.openEncode() }
            }

            Card {
                Layout.fillWidth: true
                visible: page.ctl.loaded
                title: "Chain"
                trailing: page.custom ? "Hand-edited script in use: the chain below is not applied" : page.rows.length === 0 ? "Crop only (decoded directly by FFmpeg)" : page.rows.length + (page.rows.length === 1 ? " filter" : " filters")

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8
                    Caption { text: "Source filter" }
                    Select {
                        Layout.preferredWidth: 260
                        value: page.ctl.filtersource
                        options: [["bestsource", "BestSource (accurate, default)"], ["lsmash", "L-SMASH (for awkward files)"]]
                        onPicked: (v) => page.ctl.setsource(v)
                    }
                    Item { Layout.fillWidth: true }
                }

                Repeater {
                    id: rep
                    model: page.rows
                    Rectangle {
                        id: row
                        required property var modelData
                        required property int index
                        readonly property string kind: modelData[0]
                        readonly property bool strengthBlock: ["denoise", "deband", "sharpen", "grain"].indexOf(kind) >= 0
                        Layout.fillWidth: true
                        implicitHeight: col.implicitHeight + 16
                        radius: 6
                        color: Tokens.raised
                        ColumnLayout {
                            id: col
                            anchors.fill: parent
                            anchors.margins: 8
                            anchors.leftMargin: 12
                            spacing: 8
                            RowLayout {
                                Layout.fillWidth: true
                                spacing: 8
                                Text { text: (row.index + 1) + "."; color: Tokens.dim; font.pixelSize: Tokens.body }
                                Text { text: row.modelData[1]; color: Tokens.text; font.pixelSize: Tokens.body; Layout.fillWidth: true; elide: Text.ElideRight }
                                Select {
                                    visible: row.strengthBlock
                                    Layout.preferredWidth: 120
                                    value: row.modelData[2]
                                    options: page.strengths
                                    onPicked: (v) => page.ctl.filterset(row.index, "strength", v)
                                }
                                Select {
                                    visible: row.kind === "qtgmc"
                                    Layout.preferredWidth: 140
                                    value: row.modelData[2]
                                    options: page.qtgmcPresets.map(p => [p, p])
                                    onPicked: (v) => page.ctl.filterset(row.index, "preset", v)
                                }
                                Field {
                                    visible: row.kind === "resize"
                                    Layout.preferredWidth: 70
                                    committedText: row.modelData[2].split("x")[0]
                                    validator: IntValidator { bottom: 64; top: 8192 }
                                    onCommitted: (t) => page.ctl.filterset(row.index, "width", t)
                                }
                                Text { visible: row.kind === "resize"; text: "×"; color: Tokens.dim }
                                Field {
                                    visible: row.kind === "resize"
                                    Layout.preferredWidth: 70
                                    committedText: (row.modelData[2].split("x")[1] || "")
                                    validator: IntValidator { bottom: 64; top: 8192 }
                                    onCommitted: (t) => page.ctl.filterset(row.index, "height", t)
                                }
                                IconButton { text: "▲"; enabled: row.index > 0; onClicked: page.ctl.filtermove(row.index, row.index - 1) }
                                IconButton { text: "▼"; enabled: row.index < rep.count - 1; onClicked: page.ctl.filtermove(row.index, row.index + 1) }
                                IconButton { text: "✕"; onClicked: page.ctl.filterremove(row.index) }
                            }
                            ColumnLayout {
                                visible: row.kind === "custom"
                                Layout.fillWidth: true
                                spacing: 6
                                TextArea {
                                    id: code
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: 90
                                    text: row.modelData[2]
                                    color: Tokens.text
                                    font.family: Tokens.mono
                                    font.pixelSize: 12
                                    wrapMode: TextEdit.NoWrap
                                    selectByMouse: true
                                    background: Rectangle { color: Tokens.bg; radius: 6; border.color: code.activeFocus ? Tokens.accent : Tokens.border }
                                }
                                PrimaryButton { text: "Apply code"; quiet: true; enabled: code.text !== row.modelData[2]; onClicked: page.ctl.filterset(row.index, "code", code.text) }
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
                        options: page.catalogue.map(c => [c[0], "Add " + c[1] + ": " + c[2]])
                        value: options.length ? options[0][0] : ""
                        onPicked: (v) => value = v
                    }
                    PrimaryButton { text: "Add filter"; quiet: true; onClicked: page.ctl.filteradd(addSel.value) }
                }
            }

            Card {
                Layout.fillWidth: true
                visible: page.ctl.loaded && page.plugins.length > 0
                title: "Plugins this chain needs"
                trailing: page.ctl.vsversion
                color: page.missing > 0 ? Tokens.caution : Tokens.surface
                Repeater {
                    model: page.plugins
                    RowLayout {
                        required property var modelData
                        Layout.fillWidth: true
                        spacing: 10
                        Rectangle { width: 8; height: 8; radius: 4; color: modelData[1] === "1" ? Tokens.good : modelData[1] === "0" ? "#FF6B6B" : Tokens.dim }
                        Text { text: modelData[0]; color: Tokens.text; font.family: Tokens.mono; font.pixelSize: 12; Layout.preferredWidth: 110 }
                        Text {
                            Layout.fillWidth: true
                            text: modelData[1] === "1" ? "Installed" : modelData[1] === "0" ? "Missing: " + modelData[2] : "Not checked yet"
                            color: modelData[1] === "0" ? Tokens.text : Tokens.dim
                            font.pixelSize: 12
                            wrapMode: Text.WordWrap
                        }
                    }
                }
                PrimaryButton { text: "Check again"; quiet: true; onClicked: page.ctl.pluginscan() }
            }

            Card {
                Layout.fillWidth: true
                visible: page.ctl.loaded
                title: "Script (job.vpy)"
                trailing: page.custom ? "Edited by hand" : "Generated from the chain"
                RowLayout {
                    spacing: 8
                    PrimaryButton { text: page.editing ? "Cancel editing" : (page.custom ? "Edit again" : "Edit by hand"); quiet: true; onClicked: { page.editing = !page.editing; if (page.editing) script.text = page.ctl.script } }
                    PrimaryButton { visible: page.editing; text: "Use this script"; onClicked: { page.ctl.scriptuse(script.text); page.editing = false } }
                    PrimaryButton { visible: page.custom && !page.editing; text: "Back to the generated script"; quiet: true; onClicked: page.ctl.scriptreset() }
                }
                Text {
                    Layout.fillWidth: true
                    visible: page.editing || page.custom
                    wrapMode: Text.WordWrap
                    text: "A hand-edited script replaces the chain, so later crop and filter changes do not reach it. Keep the frame count the same if the film has Dolby Vision or HDR10+, and end with clip.set_output()."
                    color: Tokens.dim
                    font.pixelSize: 12
                }
                TextArea {
                    id: script
                    Layout.fillWidth: true
                    Layout.preferredHeight: 360
                    readOnly: !page.editing
                    selectByMouse: true
                    text: page.editing ? text : page.ctl.script
                    color: Tokens.text
                    font.family: Tokens.mono
                    font.pixelSize: 12
                    wrapMode: TextEdit.NoWrap
                    background: Rectangle { color: Tokens.bg; radius: 6; border.color: page.editing ? Tokens.accent : Tokens.border }
                }
            }
        }
    }
}
