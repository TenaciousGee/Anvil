// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.anvil.app

// Drop zone when nothing is loaded; job editor once a file is open.
Item {
    id: page
    property var ctl
    property bool advanced: false
    // True while the user is choosing a different file for a loaded job.
    property bool picking: false
    // Set by the window, whose drop area covers the whole screen.
    property bool dragOver: false
    signal browseFile()
    signal browseFolder()
    signal browseSubtitle()
    // For tests and screenshots: which editor rows are open.
    property alias audioOpen: audioEd.open
    property alias subsOpen: subsEd.open
    signal importChapters()
    signal exportChapters()

    function lines(s) { return s.length ? s.split("\n") : [] }
    // Forge mode words: t(plain, forge).
    function t(plain, forge) { return Prefs.forge ? forge : plain }

    Connections {
        target: page.ctl
        function onSourceChanged() { page.picking = false }
    }

    // ---- Drop zone --------------------------------------------------------
    Item {
        anchors.fill: parent
        visible: !page.ctl.loaded || page.picking

        ColumnLayout {
            anchors.centerIn: parent
            width: Math.min(parent.width - 96, 640)
            spacing: Tokens.gap

            Banner { Layout.fillWidth: true; text: page.ctl.error; onDismissed: page.ctl.dismiss() }

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 320
                radius: 16
                color: page.dragOver ? Tokens.tint : Tokens.surface
                border.color: page.dragOver ? Tokens.accent : Tokens.border
                border.width: 1.5
                Behavior on color { ColorAnimation { duration: 150 } }

                ColumnLayout {
                    anchors.centerIn: parent
                    width: parent.width - 96
                    spacing: 14

                    Item {
                        Layout.alignment: Qt.AlignHCenter
                        implicitWidth: 44; implicitHeight: 44
                        Text { anchors.centerIn: parent; visible: !page.ctl.busy; text: "⤓"; color: Tokens.accent; font.pixelSize: 40 }
                        Spinner { anchors.centerIn: parent; running: page.ctl.busy; width: 32; height: 32 }
                    }
                    Text {
                        Layout.alignment: Qt.AlignHCenter
                        text: page.ctl.copying ? page.t("Copying the disc's main title…", "Mining the disc's main title…")
                            : page.ctl.busy ? page.t("Working…", "Stoking the fire…")
                            : page.t("Drop a film or a folder here", "Bring your raw ore")
                        color: Tokens.text; font.pixelSize: Tokens.hero; font.weight: Font.DemiBold
                    }
                    Text {
                        Layout.alignment: Qt.AlignHCenter
                        Layout.fillWidth: true
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.WordWrap
                        text: page.ctl.busy ? page.ctl.status : page.t(
                            "A film opens for checking; a folder queues every MKV or M2TS in it; a Blu-ray or DVD folder opens its main title. Quality Default: x265 10-bit, auto-crop, lossless audio then AC-3, HDR and Dolby Vision kept, at most a third of the source size.",
                            "Drop a rip to work it at the anvil, a folder to fill the smelter, or a Blu-ray or DVD to mine its main title. The house alloy: x265 10-bit, trimmed of black bars, lossless audio then AC-3, HDR and Dolby Vision kept, at most a third of the ore's weight.")
                        color: Tokens.dim
                        font.pixelSize: Tokens.body
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        Layout.topMargin: 8
                        spacing: 8
                        TextField {
                            id: pathField
                            Layout.fillWidth: true
                            enabled: !page.ctl.busy
                            placeholderText: "Or type or paste a path, e.g. ~/Rips/Film (2026)/title_t00.mkv"
                            placeholderTextColor: Tokens.dim
                            color: Tokens.text
                            selectByMouse: true
                            font.pixelSize: Tokens.body
                            background: Rectangle { implicitHeight: 36; radius: Tokens.radius; color: Tokens.bg; border.color: pathField.activeFocus ? Tokens.accent : Tokens.border }
                            onAccepted: if (text.length) page.ctl.open(text)
                        }
                        PrimaryButton { text: "Open"; enabled: pathField.text.length > 0 && !page.ctl.busy; onClicked: page.ctl.open(pathField.text) }
                    }
                    RowLayout {
                        Layout.alignment: Qt.AlignHCenter
                        spacing: 8
                        PrimaryButton { text: page.t("Browse for a film…", "Find ore…"); quiet: true; enabled: !page.ctl.busy; onClicked: page.browseFile() }
                        PrimaryButton { text: page.t("Add a folder to the queue…", "Fill the smelter from a folder…"); quiet: true; enabled: !page.ctl.busy; onClicked: page.browseFolder() }
                        PrimaryButton { text: "Cancel"; quiet: true; visible: page.picking && !page.ctl.copying; onClicked: page.picking = false }
                        PrimaryButton { text: "Stop copying"; quiet: true; danger: true; visible: page.ctl.copying; onClicked: page.ctl.stop() }
                    }
                }

            }
        }
    }

    // ---- Job editor -------------------------------------------------------
    ScrollView {
        anchors.fill: parent
        visible: page.ctl.loaded && !page.picking
        contentWidth: availableWidth
        clip: true

        ColumnLayout {
            width: parent.width
            spacing: 0

            Banner {
                Layout.fillWidth: true
                Layout.margins: 24
                Layout.bottomMargin: 0
                text: page.ctl.error
                onDismissed: page.ctl.dismiss()
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Tokens.gap

                // Left column
                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.alignment: Qt.AlignTop
                    Layout.margins: 24
                    Layout.rightMargin: 0
                    spacing: Tokens.gap

                    Card {
                        Layout.fillWidth: true
                        title: page.t("Source", "Ore")
                        RowLayout {
                            Layout.fillWidth: true
                            Text { text: page.ctl.summary; color: Tokens.text; font.pixelSize: Tokens.heading; font.weight: Font.DemiBold; Layout.fillWidth: true }
                            PrimaryButton { text: page.t("Open another", "Other ore"); quiet: true; enabled: !page.ctl.busy; onClicked: page.picking = true }
                        }
                        RowLayout {
                            Layout.fillWidth: true
                            visible: page.ctl.titles.length > 0 && page.ctl.titles.split("\n").length > 1
                            spacing: 8
                            readonly property var rows: page.ctl.titles.length ? page.ctl.titles.split("\n").map(l => l.split("\t")) : []
                            Caption { text: "Disc title" }
                            Select {
                                Layout.fillWidth: true
                                enabled: !page.ctl.busy
                                options: parent.rows.map(r => [r[0], r[1] + (r[2] === "1" ? "  · main title" : "")])
                                value: (parent.rows.find(r => r[3] === "1") || [""])[0]
                                // Choosing another title copies it and opens it.
                                onPicked: (v) => { if (v !== value) page.ctl.opentitle(v) }
                            }
                        }
                        Text {
                            Layout.fillWidth: true
                            visible: page.ctl.origin.length > 0
                            text: page.ctl.origin
                            color: Tokens.dim
                            font.pixelSize: 12
                            wrapMode: Text.WordWrap
                        }
                        Flow {
                            Layout.fillWidth: true
                            spacing: 8
                            Repeater {
                                model: page.lines(page.ctl.badges)
                                Chip {
                                    required property string modelData
                                    text: modelData
                                    tone: modelData.indexOf("Dolby Vision") >= 0 || modelData.indexOf("HDR") >= 0 ? Tokens.warn : Tokens.accent
                                }
                            }
                        }
                        Text {
                            Layout.fillWidth: true
                            visible: page.ctl.rules.length > 0
                            text: page.t("Preset rules applied: ", "Alloy rules applied: ") + page.lines(page.ctl.rules).join(", ")
                            color: Tokens.dim
                            font.pixelSize: 12
                            wrapMode: Text.WordWrap
                        }
                    }

                    Card {
                        Layout.fillWidth: true
                        title: "Picture"
                        trailing: page.ctl.crop
                        PreviewPane { id: preview; Layout.fillWidth: true; ctl: page.ctl }
                    }

                    AudioEditor { id: audioEd; Layout.fillWidth: true; ctl: page.ctl }
                    SubtitleEditor { id: subsEd; Layout.fillWidth: true; ctl: page.ctl; onAddFile: page.browseSubtitle() }
                    ChapterEditor {
                        Layout.fillWidth: true
                        ctl: page.ctl
                        position: preview.position
                        onImportFile: page.importChapters()
                        onExportFile: page.exportChapters()
                    }
                }

                // Right column
                ColumnLayout {
                    Layout.preferredWidth: 380
                    Layout.maximumWidth: 420
                    Layout.alignment: Qt.AlignTop
                    Layout.margins: 24
                    Layout.leftMargin: 0
                    spacing: Tokens.gap

                    Card {
                        Layout.fillWidth: true
                        title: page.t("Preset", "Alloy")
                        ComboBox {
                            id: presetBox
                            Layout.fillWidth: true
                            // Only presets this machine can encode with (and the one in use).
                            readonly property var entries: page.lines(page.ctl.presets).map(l => l.split("\t"))
                                .filter(e => !e[4] || e[0] === page.ctl.preset)
                            model: entries.map(e => e[1])
                            currentIndex: Math.max(0, entries.findIndex(e => e[0] === page.ctl.preset))
                            onActivated: (i) => page.ctl.choose(entries[i][0])
                            contentItem: Text { leftPadding: 12; text: presetBox.displayText; color: Tokens.text; font.pixelSize: Tokens.body; verticalAlignment: Text.AlignVCenter }
                            background: Rectangle { implicitHeight: 36; radius: Tokens.radius; color: Tokens.raised; border.color: Tokens.border }
                        }
                        Text {
                            Layout.fillWidth: true
                            wrapMode: Text.WordWrap
                            text: (presetBox.entries[presetBox.currentIndex] || ["", "", ""])[2]
                            color: Tokens.dim
                            font.pixelSize: 12
                        }
                    }

                    Card {
                        Layout.fillWidth: true
                        title: page.t("How long will it take?", "How long in the forge?")
                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 10
                            PrimaryButton {
                                text: page.ctl.estimating ? "Stop" : (page.ctl.estimate.length ? "Estimate again" : "Estimate time")
                                quiet: true
                                enabled: !page.ctl.busy
                                onClicked: page.ctl.estimating ? page.ctl.estimatestop() : page.ctl.estimatetime()
                            }
                            Text {
                                Layout.fillWidth: true
                                text: page.ctl.estimate.length ? page.ctl.estimate
                                      : "Times a few short encodes with these settings on this machine (about a minute)."
                                color: page.ctl.estimate.length && !page.ctl.estimating ? Tokens.text : Tokens.dim
                                font.pixelSize: page.ctl.estimate.length && !page.ctl.estimating ? Tokens.body : 12
                                wrapMode: Text.WordWrap
                            }
                        }
                    }

                    Card {
                        Layout.fillWidth: true
                        title: page.t("Before you queue", "Before it goes in the smelter")
                        visible: page.ctl.warnings.length > 0
                        color: Tokens.caution
                        border.color: "#59F2B35B"
                        Repeater {
                            model: page.lines(page.ctl.warnings)
                            Text {
                                required property string modelData
                                Layout.fillWidth: true
                                text: "•  " + modelData
                                color: Tokens.text
                                wrapMode: Text.WordWrap
                                font.pixelSize: 12
                            }
                        }
                    }

                    Card {
                        Layout.fillWidth: true
                        title: page.t("Steps", "Forging steps")
                        trailing: page.lines(page.ctl.steps).length + " steps" + (page.advanced ? "" : " · commands in Advanced")
                        Repeater {
                            model: page.lines(page.ctl.steps)
                            ColumnLayout {
                                id: stepRow
                                required property string modelData
                                required property int index
                                Layout.fillWidth: true
                                spacing: 4
                                Text {
                                    text: (stepRow.index + 1) + "  " + stepRow.modelData.split("\t")[0]
                                    color: Tokens.text
                                    font.pixelSize: Tokens.body
                                    Layout.fillWidth: true
                                    elide: Text.ElideRight
                                }
                                TextEdit {
                                    visible: page.advanced
                                    text: stepRow.modelData.split("\t")[1] || ""
                                    readOnly: true
                                    selectByMouse: true
                                    color: Tokens.dim
                                    font.family: Tokens.mono
                                    font.pixelSize: 11
                                    wrapMode: Text.WrapAnywhere
                                    Layout.fillWidth: true
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- The whole page takes drops ---------------------------------------
    // Over the empty page the card lights up as before; over an open job a
    // light overlay says the drop replaces it.
    Rectangle {
        anchors.fill: parent
        anchors.margins: 12
        visible: page.dragOver && page.ctl.loaded && !page.picking
        radius: 16
        color: "transparent"
        border.color: Tokens.accent
        border.width: 1.5
        Rectangle { anchors.fill: parent; radius: 16; color: Tokens.bg; opacity: 0.92 }
        Rectangle { anchors.fill: parent; radius: 16; color: Tokens.tint; border.color: Tokens.accent; border.width: 1.5 }
        Column {
            anchors.centerIn: parent
            spacing: 8
            Text { anchors.horizontalCenter: parent.horizontalCenter; text: "⤓"; color: Tokens.accent; font.pixelSize: 40 }
            Text {
                anchors.horizontalCenter: parent.horizontalCenter
                text: page.t("Drop to open instead", "Drop to swap the ore")
                color: Tokens.text; font.pixelSize: Tokens.hero; font.weight: Font.DemiBold
            }
            Text {
                anchors.horizontalCenter: parent.horizontalCenter
                text: "A folder of films goes to the queue; the open job stays as it is until a film opens."
                color: Tokens.dim; font.pixelSize: Tokens.body
            }
        }
    }
}
