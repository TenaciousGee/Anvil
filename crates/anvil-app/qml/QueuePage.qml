// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.anvil.app

// The persistent job queue with live progress.
ScrollView {
    id: page
    property var queue
    signal openEncode()

    contentWidth: availableWidth
    clip: true

    function lines(s) { return s.length ? s.split("\n") : [] }
    // Forge mode words: t(plain, forge).
    function t(plain, forge) { return Prefs.forge ? forge : plain }
    // In Forge mode, what a live step is doing to the metal.
    function stage(step) {
        if (!Prefs.forge) return step
        if (/^Encode video/.test(step)) return "Hammering · " + step
        if (/^(Mux|Inject|Verify)/.test(step)) return "Tempering · " + step
        return "Smelting · " + step
    }
    readonly property var rows: lines(queue.rows).map(l => l.split("\t"))
    // id -> [fraction, step, detail, "done/total"]
    readonly property var liveById: {
        const m = {}
        for (const l of lines(queue.live)) { const f = l.split("\t"); m[f[0]] = f.slice(1) }
        return m
    }
    readonly property int finished: rows.filter(r => ["done", "cancelled"].indexOf(r[3]) >= 0).length

    function statusColour(s) {
        return s === "running" ? Tokens.accent
             : s === "paused" || s === "interrupted" ? Tokens.warn
             : s === "done" ? Tokens.good
             : s === "failed" ? "#FF6B6B"
             : Tokens.dim
    }
    function statusLabel(s) {
        return (Prefs.forge
            ? { pending: "Heating", running: "Hammering", paused: "Banked", done: "Tempered", failed: "Cracked",
                cancelled: "Quenched", interrupted: "Gone cold" }
            : { pending: "Waiting", running: "Encoding", paused: "Paused", done: "Done", failed: "Failed",
                cancelled: "Cancelled", interrupted: "Interrupted" })[s] || s
    }

    ColumnLayout {
        width: page.availableWidth
        spacing: 0

        ColumnLayout {
            Layout.fillWidth: true
            Layout.margins: 24
            spacing: Tokens.gap

            RowLayout {
                Layout.fillWidth: true
                spacing: 8
                PageHeader {
                    Layout.fillWidth: true
                    title: page.t("Queue", "Smelter")
                    subtitle: page.rows.length === 0 ? page.t("Nothing queued yet.", "The smelter is cold.")
                        : (page.queue.active > 0 ? page.queue.active + page.t(" encoding · ", " under the hammer · ") : "")
                          + page.queue.waiting + page.t(" waiting", " heating")
                          + (page.queue.held && page.queue.waiting > 0 ? page.t(" · queue paused, press Start", " · fire banked, press Light the fire") : "")
                }
                PrimaryButton { text: page.t("Clear finished", "Clear the tempered"); quiet: true; visible: page.finished > 0; onClicked: page.queue.clear() }
                PrimaryButton {
                    text: page.queue.held ? page.t("Start queue", "Light the fire") : page.t("Pause queue", "Bank the fire")
                    quiet: !page.queue.held
                    enabled: page.queue.held ? page.queue.waiting > 0 : true
                    onClicked: page.queue.held ? page.queue.start() : page.queue.hold()
                }
            }

            Banner { Layout.fillWidth: true; text: page.queue.notice; onDismissed: page.queue.refresh() }

            // Empty state
            Card {
                Layout.fillWidth: true
                visible: page.rows.length === 0
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 12
                    Text { text: page.t("Your queue is empty", "The smelter is empty"); color: Tokens.text; font.pixelSize: Tokens.heading; font.weight: Font.DemiBold }
                    Text {
                        Layout.fillWidth: true
                        wrapMode: Text.WordWrap
                        text: page.t("Open a film on the Encode page and press Encode, or drop a whole folder there to queue every film in it. The queue is saved, so jobs survive restarts.",
                                     "Bring ore to the Forge and press Hammer it, or drop a whole folder there to fill the smelter. The smelter keeps its fire banked between sessions, so jobs survive restarts.")
                        color: Tokens.dim
                        font.pixelSize: Tokens.body
                    }
                    PrimaryButton { text: page.t("Go to Encode", "Go to the Forge"); onClicked: page.openEncode() }
                }
            }

            Repeater {
                model: page.rows
                Card {
                    id: jobCard
                    required property var modelData
                    required property int index
                    readonly property string jobId: modelData[0]
                    readonly property string status: modelData[3]
                    readonly property var live: page.liveById[jobId]
                    readonly property bool activeJob: status === "running" || status === "paused"
                    property bool showChecks: false
                    Layout.fillWidth: true

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 16

                        // Thumbnail
                        Rectangle {
                            Layout.preferredWidth: 160
                            Layout.preferredHeight: 90
                            Layout.alignment: Qt.AlignTop
                            radius: 6
                            color: "#07080A"
                            border.color: Tokens.border
                            clip: true
                            Image {
                                anchors.fill: parent
                                anchors.margins: 1
                                source: jobCard.modelData[6]
                                fillMode: Image.PreserveAspectCrop
                                asynchronous: true
                                visible: status === Image.Ready
                            }
                            Text { anchors.centerIn: parent; text: "🎞"; font.pixelSize: 24; color: Tokens.dim; visible: jobCard.modelData[6].length === 0 }
                        }

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 6
                            RowLayout {
                                Layout.fillWidth: true
                                spacing: 8
                                Text { text: jobCard.modelData[1]; color: Tokens.text; font.pixelSize: Tokens.heading; font.weight: Font.DemiBold; elide: Text.ElideMiddle; Layout.fillWidth: true }
                                Chip { text: page.statusLabel(jobCard.status); tone: page.statusColour(jobCard.status) }
                            }
                            Text {
                                Layout.fillWidth: true
                                text: jobCard.modelData[2] + (jobCard.modelData[4].length ? "  ·  " + jobCard.modelData[4] : "")
                                color: Tokens.dim; font.pixelSize: 12; elide: Text.ElideRight
                            }
                            Text { Layout.fillWidth: true; text: "→ " + jobCard.modelData[5]; color: Tokens.dim; font.pixelSize: 11; elide: Text.ElideMiddle }

                            // Live progress
                            ColumnLayout {
                                Layout.fillWidth: true
                                Layout.topMargin: 4
                                visible: jobCard.activeJob
                                spacing: 6
                                Rectangle {
                                    Layout.fillWidth: true
                                    height: 6; radius: 3
                                    color: Tokens.raised
                                    Rectangle {
                                        width: parent.width * (jobCard.live ? parseFloat(jobCard.live[0]) : 0)
                                        height: parent.height; radius: 3
                                        color: jobCard.status === "paused" ? Tokens.warn : Tokens.accent
                                        Behavior on width { NumberAnimation { duration: 300 } }
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Text {
                                        text: jobCard.live ? Math.round(parseFloat(jobCard.live[0]) * 100) + "%  ·  " + page.stage(jobCard.live[1]) : page.t("Starting…", "Heating…")
                                        color: Tokens.text; font.pixelSize: 12; font.weight: Font.DemiBold
                                    }
                                    Text {
                                        Layout.fillWidth: true
                                        text: jobCard.live && jobCard.live[2].length ? jobCard.live[2] : ""
                                        color: Tokens.dim; font.pixelSize: 12; elide: Text.ElideRight
                                    }
                                    Text { text: jobCard.live ? jobCard.live[3].replace("/", " of ") + page.t(" steps done", " blows struck") : ""; color: Tokens.dim; font.pixelSize: 11 }
                                }
                            }

                            // Result / problem
                            Text {
                                Layout.fillWidth: true
                                visible: !jobCard.activeJob && (jobCard.modelData[7].length > 0 || jobCard.modelData[8].length > 0)
                                text: jobCard.status === "done"
                                    ? "✔ " + page.t("", "Assay: ") + jobCard.modelData[8] + (jobCard.modelData[7].length ? "  ·  " + jobCard.modelData[7] : "")
                                    : jobCard.modelData[7].split("\n")[0]
                                color: jobCard.status === "done" ? Tokens.good : (jobCard.status === "failed" ? "#FF8A8A" : Tokens.dim)
                                font.pixelSize: 12
                                wrapMode: Text.WordWrap
                                maximumLineCount: 3
                                elide: Text.ElideRight
                            }
                            TextEdit {
                                Layout.fillWidth: true
                                visible: jobCard.showChecks
                                readOnly: true
                                selectByMouse: true
                                text: jobCard.showChecks ? page.queue.checks(parseInt(jobCard.jobId)) : ""
                                color: Tokens.text
                                font.family: Tokens.mono
                                font.pixelSize: 11
                                wrapMode: Text.WrapAnywhere
                            }
                        }

                        // Actions
                        ColumnLayout {
                            Layout.alignment: Qt.AlignTop
                            spacing: 6
                            readonly property int jid: parseInt(jobCard.jobId)
                            PrimaryButton { Layout.fillWidth: true; text: "Pause"; quiet: true; visible: jobCard.status === "running"; onClicked: page.queue.pause(parent.jid) }
                            PrimaryButton { Layout.fillWidth: true; text: "Resume"; visible: jobCard.status === "paused"; onClicked: page.queue.resume(parent.jid) }
                            PrimaryButton { Layout.fillWidth: true; text: "Cancel"; quiet: true; danger: true; visible: jobCard.activeJob || jobCard.status === "pending"; onClicked: page.queue.cancel(parent.jid) }
                            PrimaryButton { Layout.fillWidth: true; text: "Retry"; visible: ["failed", "cancelled", "interrupted"].indexOf(jobCard.status) >= 0; onClicked: page.queue.retry(parent.jid) }
                            PrimaryButton { Layout.fillWidth: true; text: "Show in folder"; quiet: true; visible: jobCard.status === "done"; onClicked: page.queue.reveal(parent.jid) }
                            PrimaryButton {
                                Layout.fillWidth: true
                                text: jobCard.showChecks ? page.t("Hide details", "Hide assay") : page.t("Details", "Assay")
                                quiet: true
                                visible: ["done", "failed", "interrupted", "cancelled"].indexOf(jobCard.status) >= 0
                                onClicked: jobCard.showChecks = !jobCard.showChecks
                            }
                            RowLayout {
                                visible: !jobCard.activeJob
                                spacing: 4
                                IconButton { text: "▲"; enabled: jobCard.index > 0; onClicked: page.queue.up(parseInt(jobCard.jobId)) }
                                IconButton { text: "▼"; enabled: jobCard.index < page.rows.length - 1; onClicked: page.queue.down(parseInt(jobCard.jobId)) }
                                IconButton { text: "✕"; onClicked: page.queue.remove(parseInt(jobCard.jobId)) }
                            }
                        }
                    }
                }
            }
        }
    }
}
