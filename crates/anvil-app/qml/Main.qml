// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Window
import QtQuick.Dialogs
import com.anvil.app

ApplicationWindow {
    id: window
    width: 1280
    height: 820
    minimumWidth: 1000
    minimumHeight: 640
    visible: true
    title: "Anvil"
    color: Tokens.bg

    property bool advanced: false
    // 0 Encode, 1 Queue, 2 Presets, 3 Filters, 4 Tools
    property int page: 0
    // Forge mode words: t(plain, forge).
    function t(plain, forge) { return Prefs.forge ? forge : plain }
    readonly property var pageNames: Prefs.forge ? ["Forge", "Smelter", "Alloys", "Flux", "Tongs"]
                                                 : ["Encode", "Queue", "Presets", "Filters", "Tools"]

    // Forge mode warms the accent to ember.
    Binding { target: Tokens; property: "accent"; value: Prefs.forge ? "#FF8A3D" : "#7C9CFF" }
    Binding { target: Tokens; property: "tint"; value: Prefs.forge ? "#26FF8A3D" : "#247C9CFF" }
    readonly property int queued: queue.active + queue.waiting
    readonly property int missingTools: job.tools.split("\n").filter(r => r.split("\t")[1] === "missing").length

    JobController { id: job }
    QueueController { id: queue }

    // Native dialogs (the XDG desktop portal is used where the platform
    // provides it, e.g. under Flatpak or with QT_QPA_PLATFORMTHEME=xdgdesktopportal).
    FileDialog {
        id: fileDialog
        title: "Choose a film"
        nameFilters: ["Video files (*.mkv *.m2ts *.mts *.mp4 *.ts)", "Blu-ray or DVD (index.bdmv VIDEO_TS.IFO)", "All files (*)"]
        onAccepted: job.open(selectedFile.toString())
    }
    FileDialog {
        id: subDialog
        title: "Add a subtitle file"
        nameFilters: ["Subtitles (*.srt *.ass *.ssa *.sup *.idx)", "All files (*)"]
        onAccepted: job.subfile(selectedFile.toString())
    }
    FileDialog {
        id: chapterOpen
        title: "Import chapters"
        nameFilters: ["Chapters (*.xml *.txt)", "All files (*)"]
        onAccepted: job.chapimport(selectedFile.toString())
    }
    FileDialog {
        id: chapterSave
        title: "Export chapters (.xml for Matroska XML, .txt for OGM)"
        fileMode: FileDialog.SaveFile
        defaultSuffix: "xml"
        nameFilters: ["Matroska XML (*.xml)", "OGM text (*.txt)"]
        onAccepted: job.chapexport(selectedFile.toString())
    }
    FolderDialog {
        id: folderDialog
        title: "Add every film in a folder to the queue, or open a Blu-ray or DVD folder"
        onAccepted: job.batch(selectedFolder.toString())
    }

    // Encode: queue the job and start the queue straight away.
    function encodeNow() {
        if (job.enqueue()) {
            queue.start()
            window.page = 1
        }
    }

    Rectangle {
        id: canvas
        anchors.fill: parent
        color: Tokens.bg

        RowLayout {
            anchors.fill: parent
            spacing: 0

            // ---- Navigation rail ------------------------------------------
            Rectangle {
                Layout.fillHeight: true
                Layout.preferredWidth: 72
                color: Tokens.rail

                Column {
                    anchors.top: parent.top
                    anchors.topMargin: 16
                    anchors.horizontalCenter: parent.horizontalCenter
                    spacing: 4

                    Image {
                        anchors.horizontalCenter: parent.horizontalCenter
                        width: 44; height: 44
                        source: "qrc:/qt/qml/com/anvil/app/assets/anvil.png"
                        sourceSize: Qt.size(88, 88)
                        smooth: true
                        mipmap: true
                    }
                    Item { width: 1; height: 16 }
                    NavButton { glyph: "◉"; label: window.pageNames[0]; active: window.page === 0; onClicked: window.page = 0 }
                    NavButton { glyph: "☰"; label: window.queued ? window.pageNames[1] + " (" + window.queued + ")" : window.pageNames[1]; active: window.page === 1; onClicked: window.page = 1 }
                    NavButton { glyph: "◇"; label: window.pageNames[2]; active: window.page === 2; onClicked: window.page = 2 }
                    NavButton { glyph: "≋"; label: window.pageNames[3]; active: window.page === 3; onClicked: window.page = 3 }
                    NavButton {
                        glyph: "⚙"; label: window.pageNames[4]; active: window.page === 4; onClicked: window.page = 4
                        // Red dot when required tools are missing.
                        Rectangle { visible: window.missingTools > 0; width: 8; height: 8; radius: 4; color: "#FF6B6B"; x: 46; y: 12 }
                    }
                }
            }

            // ---- Main area ------------------------------------------------
            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 0

                // Header
                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 64
                    color: Tokens.bg

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 24
                        anchors.rightMargin: 24
                        spacing: 16

                        ColumnLayout {
                            spacing: 2
                            Layout.fillWidth: true
                            Text {
                                Layout.fillWidth: true
                                text: window.page === 0 && job.loaded ? job.source.split("/").pop() : window.pageNames[window.page]
                                color: Tokens.text; font.pixelSize: Tokens.heading; font.weight: Font.DemiBold
                                elide: Text.ElideMiddle
                            }
                            RowLayout {
                                spacing: 6
                                Spinner { running: job.busy; implicitWidth: 12; implicitHeight: 12 }
                                Text {
                                    Layout.fillWidth: true
                                    text: job.status
                                    color: job.busy ? Tokens.accent : Tokens.dim
                                    font.pixelSize: 12
                                    elide: Text.ElideRight
                                }
                            }
                        }

                        // Simple / Advanced segmented switch (Encode page only)
                        Rectangle {
                            visible: window.page === 0 && job.loaded
                            implicitWidth: seg.implicitWidth + 8
                            implicitHeight: 36
                            radius: Tokens.radius
                            color: Tokens.surface
                            border.color: Tokens.border
                            Row {
                                id: seg
                                anchors.centerIn: parent
                                spacing: 4
                                Repeater {
                                    model: ["Simple", "Advanced"]
                                    Rectangle {
                                        id: segItem
                                        required property string modelData
                                        required property int index
                                        readonly property bool on: (index === 1) === window.advanced
                                        width: segLabel.implicitWidth + 24
                                        height: 28
                                        radius: 6
                                        color: on ? Tokens.raised : "transparent"
                                        Text { id: segLabel; anchors.centerIn: parent; text: segItem.modelData; color: segItem.on ? Tokens.text : Tokens.dim; font.pixelSize: 12; font.weight: Font.DemiBold }
                                        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: window.advanced = (segItem.index === 1) }
                                    }
                                }
                            }
                        }

                        PrimaryButton {
                            visible: window.page === 0 && job.loaded
                            text: window.t("Add to queue", "To the smelter"); quiet: true
                            enabled: !job.busy && job.steps.length > 0
                            onClicked: job.enqueue()
                        }
                        PrimaryButton {
                            visible: window.page === 0 && job.loaded
                            text: window.t("Encode", "Hammer it")
                            enabled: !job.busy && job.steps.length > 0
                            // Queue it and save its run script straight away.
                            onClicked: window.encodeNow()
                        }
                    }
                }

                Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Tokens.border }

                StackLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    currentIndex: window.page

                    EncodePage {
                        id: encodePage
                        ctl: job
                        dragOver: screenDrop.containsDrag
                        advanced: window.advanced
                        onBrowseFile: fileDialog.open()
                        onBrowseFolder: folderDialog.open()
                        onBrowseSubtitle: subDialog.open()
                        onImportChapters: chapterOpen.open()
                        onExportChapters: chapterSave.open()
                    }
                    QueuePage { queue: queue; onOpenEncode: window.page = 0 }
                    PresetsPage { id: presetsPage; ctl: job }
                    FiltersPage { ctl: job; onOpenEncode: window.page = 0 }
                    ToolsPage { ctl: job }
                }
            }
        }
    }

    // ---- Drops anywhere on the Encode screen (rail and header included) ----
    DropArea {
        id: screenDrop
        anchors.fill: parent
        enabled: window.page === 0 && !job.busy
        // The controller decodes file:// URLs (spaces arrive as %20).
        onDropped: (d) => {
            if (!d.hasUrls)
                return
            d.acceptProposedAction()
            job.open(d.urls[0].toString())
        }
    }

    // ---- Developer hooks (also used for automated tests and screenshots) -----
    //   --preset ID  --open FILE  --load-probe JSON  --batch DIR  --advanced  --page N
    //   --forge  --size WxH  --js CODE (job, window and page in scope)
    //   --encode            press Encode once the file is ready (auto-crop done)
    //   --screenshot PNG    capture after --shot-delay ms (default 3000)
    //   --shots DIR         with --encode: capture DIR/queue-NN.png every 5 s
    //   --quit-when-done    with --encode: quit once the queue is idle
    readonly property var args: Qt.application.arguments
    function arg(flag) { const i = args.indexOf(flag); return i >= 0 && i + 1 < args.length ? args[i + 1] : "" }
    function has(flag) { return args.indexOf(flag) >= 0 }
    property bool encodeRequested: has("--encode")
    property bool encodeStarted: false
    property int shotIndex: 0

    property alias encodePage: encodePage
    property alias presetsPage: presetsPage
    Timer {
        // --js CODE: run once the file is ready (job, window and page in scope).
        interval: 250; repeat: true
        running: window.arg("--js").length > 0
        onTriggered: if (job.loaded && !job.busy && job.steps.length > 0) {
            running = false
            new Function("job", "window", "page", window.arg("--js"))(job, window, encodePage)
        }
    }
    Component.onCompleted: {
        if (arg("--size")) {
            const wh = arg("--size").split("x")
            window.width = parseInt(wh[0]); window.height = parseInt(wh[1])
        }
        if (arg("--preset")) job.choose(arg("--preset"))
        if (arg("--open")) job.open(arg("--open"))
        if (arg("--load-probe")) job.load(arg("--load-probe"))
        if (arg("--batch")) job.batch(arg("--batch"))
        if (has("--advanced")) window.advanced = true
        // Forge mode for this run only (not saved).
        if (has("--forge")) Prefs.forge = true
        if (arg("--page")) window.page = parseInt(arg("--page"))
    }
    Timer {
        // --preview-filtered: render the exact after-filters frame once ready.
        interval: 250; repeat: true
        running: window.has("--preview-filtered")
        onTriggered: if (job.loaded && !job.busy && job.steps.length > 0) { running = false; job.preview(0.3, true) }
    }
    Timer {
        // Press Encode once the file is open and auto-crop has finished.
        interval: 250; repeat: true
        running: window.encodeRequested && !window.encodeStarted
        onTriggered: {
            if (job.loaded && !job.busy && job.steps.length > 0) {
                window.encodeStarted = true
                window.encodeNow()
            }
        }
    }
    Timer {
        interval: 5000; repeat: true
        running: window.has("--shots") && window.encodeStarted
        onTriggered: {
            const n = ("0" + (++window.shotIndex)).slice(-2)
            canvas.grabToImage(function (r) { r.saveToFile(window.arg("--shots") + "/queue-" + n + ".png") })
        }
    }
    Timer {
        interval: 1000; repeat: true
        running: window.has("--quit-when-done") && window.encodeStarted
        onTriggered: {
            if (queue.active === 0 && queue.waiting === 0) {
                const out = window.arg("--screenshot")
                if (out) canvas.grabToImage(function (r) { r.saveToFile(out); Qt.quit() })
                else Qt.quit()
            }
        }
    }
    Timer {
        interval: window.arg("--shot-delay") ? parseInt(window.arg("--shot-delay")) : 3000
        running: window.has("--screenshot") && !window.has("--quit-when-done")
        onTriggered: canvas.grabToImage(function (r) { r.saveToFile(window.arg("--screenshot")); Qt.quit() })
    }
}
