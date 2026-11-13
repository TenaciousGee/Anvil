// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.anvil.app

// Frame preview with crop editing.
//  - Source: the untouched frame, with the crop drawn over it. Drag any edge
//    to adjust; values snap to even pixels and are checked by the engine.
//  - After filters: the exact frame from the job's VapourSynth script, as it
//    will be encoded (already cropped and filtered).
ColumnLayout {
    id: pane
    property var ctl
    property bool filtered: false
    property real position: 0.3
    spacing: 10

    function request() { pane.ctl.preview(pane.position, pane.filtered) }
    function step(seconds) {
        if (pane.ctl.duration <= 0) return
        pane.position = Math.max(0, Math.min(1, pane.position + seconds / pane.ctl.duration))
        request()
    }

    // An after-filters frame is stale once the crop changes.
    Connections {
        target: pane.ctl
        function onCropChanged() { if (pane.filtered) pane.request() }
        function onSourceChanged() { pane.position = 0.3; pane.filtered = false }
    }

    // ---- Frame ---------------------------------------------------------------
    Rectangle {
        id: stage
        Layout.fillWidth: true
        Layout.preferredHeight: Math.min(width * 9 / 16, 420)
        radius: 6
        color: "#07080A"
        border.color: Tokens.border
        clip: true

        Image {
            id: img
            anchors.fill: parent
            anchors.margins: 1
            source: pane.ctl.frame
            fillMode: Image.PreserveAspectFit
            asynchronous: true
            cache: false
            smooth: true
        }

        Column {
            anchors.centerIn: parent
            spacing: 8
            visible: pane.ctl.frame.length === 0
            Spinner { anchors.horizontalCenter: parent.horizontalCenter; running: pane.ctl.framebusy; width: 24; height: 24 }
            Text {
                anchors.horizontalCenter: parent.horizontalCenter
                text: pane.ctl.framebusy ? "Rendering a frame…" : "No preview yet"
                color: Tokens.dim
                font.pixelSize: 12
            }
        }

        // ---- Crop overlay (source frames only) -----------------------------
        Item {
            id: overlay
            anchors.fill: img
            visible: pane.ctl.framesource && img.status === Image.Ready && pane.ctl.srcw > 0

            // Displayed picture rectangle inside the Image.
            readonly property real ox: (width - img.paintedWidth) / 2
            readonly property real oy: (height - img.paintedHeight) / 2
            readonly property real s: img.paintedWidth / Math.max(1, pane.ctl.srcw)

            // While dragging, the edited values; otherwise the engine's.
            property int dl: -1; property int dr: -1; property int dt: -1; property int db: -1
            readonly property int cl: dl >= 0 ? dl : pane.ctl.cropl
            readonly property int cr: dr >= 0 ? dr : pane.ctl.cropr
            readonly property int ct: dt >= 0 ? dt : pane.ctl.cropt
            readonly property int cb: db >= 0 ? db : pane.ctl.cropb

            readonly property real x0: ox + cl * s
            readonly property real x1: ox + img.paintedWidth - cr * s
            readonly property real y0: oy + ct * s
            readonly property real y1: oy + img.paintedHeight - cb * s

            function commit() {
                pane.ctl.setcrop(cl, cr, ct, cb)
                dl = dr = dt = db = -1
            }

            // Shade what will be cut away.
            Rectangle { x: overlay.ox; y: overlay.oy; width: img.paintedWidth; height: overlay.y0 - overlay.oy; color: "#B0000000" }
            Rectangle { x: overlay.ox; y: overlay.y1; width: img.paintedWidth; height: overlay.oy + img.paintedHeight - overlay.y1; color: "#B0000000" }
            Rectangle { x: overlay.ox; y: overlay.y0; width: overlay.x0 - overlay.ox; height: overlay.y1 - overlay.y0; color: "#B0000000" }
            Rectangle { x: overlay.x1; y: overlay.y0; width: overlay.ox + img.paintedWidth - overlay.x1; height: overlay.y1 - overlay.y0; color: "#B0000000" }
            Rectangle {
                x: overlay.x0; y: overlay.y0
                width: overlay.x1 - overlay.x0; height: overlay.y1 - overlay.y0
                color: "transparent"
                border.color: Tokens.accent
                border.width: 1.5
            }

            // Edge handles: drag to crop.
            component Handle: Rectangle {
                id: h
                property bool vertical: false   // a left/right edge
                signal dragged(real px, real py)
                width: vertical ? 10 : 44
                height: vertical ? 44 : 10
                radius: 5
                color: area.pressed ? Tokens.text : Tokens.accent
                border.color: Tokens.bg
                MouseArea {
                    id: area
                    anchors.fill: parent
                    anchors.margins: -10
                    cursorShape: h.vertical ? Qt.SizeHorCursor : Qt.SizeVerCursor
                    preventStealing: true
                    onPositionChanged: (m) => { const p = mapToItem(overlay, m.x, m.y); h.dragged(p.x, p.y) }
                    onReleased: overlay.commit()
                }
            }
            readonly property real midX: (x0 + x1) / 2
            readonly property real midY: (y0 + y1) / 2
            readonly property int minSize: 64

            Handle {
                x: overlay.midX - width / 2; y: overlay.y0 - height / 2
                onDragged: (px, py) => overlay.dt = Math.max(0, Math.min(Math.round((py - overlay.oy) / overlay.s), pane.ctl.srch - overlay.cb - overlay.minSize))
            }
            Handle {
                x: overlay.midX - width / 2; y: overlay.y1 - height / 2
                onDragged: (px, py) => overlay.db = Math.max(0, Math.min(Math.round((overlay.oy + img.paintedHeight - py) / overlay.s), pane.ctl.srch - overlay.ct - overlay.minSize))
            }
            Handle {
                vertical: true
                x: overlay.x0 - width / 2; y: overlay.midY - height / 2
                onDragged: (px, py) => overlay.dl = Math.max(0, Math.min(Math.round((px - overlay.ox) / overlay.s), pane.ctl.srcw - overlay.cr - overlay.minSize))
            }
            Handle {
                vertical: true
                x: overlay.x1 - width / 2; y: overlay.midY - height / 2
                onDragged: (px, py) => overlay.dr = Math.max(0, Math.min(Math.round((overlay.ox + img.paintedWidth - px) / overlay.s), pane.ctl.srcw - overlay.cl - overlay.minSize))
            }

            // Live size readout while editing.
            Rectangle {
                visible: overlay.dl >= 0 || overlay.dr >= 0 || overlay.dt >= 0 || overlay.db >= 0
                anchors.centerIn: parent
                radius: 6
                color: "#D0101318"
                width: sizeText.implicitWidth + 20
                height: sizeText.implicitHeight + 12
                Text {
                    id: sizeText
                    anchors.centerIn: parent
                    color: Tokens.text
                    font.pixelSize: 13
                    font.weight: Font.DemiBold
                    text: (pane.ctl.srcw - overlay.cl - overlay.cr) + " × " + (pane.ctl.srch - overlay.ct - overlay.cb)
                }
            }
        }

        // Busy badge while a new frame renders over an old one.
        Rectangle {
            visible: pane.ctl.framebusy && pane.ctl.frame.length > 0
            anchors.top: parent.top; anchors.right: parent.right; anchors.margins: 10
            radius: 12; height: 26; width: busyRow.implicitWidth + 18
            color: "#D0101318"
            Row {
                id: busyRow
                anchors.centerIn: parent
                spacing: 6
                Spinner { running: true; implicitWidth: 12; implicitHeight: 12; anchors.verticalCenter: parent.verticalCenter }
                Text { text: pane.filtered ? "Rendering exact frame…" : "Seeking…"; color: Tokens.text; font.pixelSize: 11; anchors.verticalCenter: parent.verticalCenter }
            }
        }
        Text {
            visible: pane.ctl.frameinfo.length > 0
            anchors.left: parent.left; anchors.bottom: parent.bottom; anchors.margins: 10
            text: pane.ctl.frameinfo + (pane.ctl.hdr ? " · HDR shown tone-mapped" : "")
            color: Tokens.text
            font.pixelSize: 11
            style: Text.Outline
            styleColor: "#000000"
        }
    }

    // ---- Scrub bar -----------------------------------------------------------
    RowLayout {
        Layout.fillWidth: true
        spacing: 8
        IconButton { text: "⏪"; enabled: pane.ctl.loaded; onClicked: pane.step(-60) }
        IconButton { text: "◀"; enabled: pane.ctl.loaded; onClicked: pane.step(-10) }
        Slider {
            id: scrub
            Layout.fillWidth: true
            from: 0; to: 1
            value: pane.position
            onMoved: pane.position = value
            onPressedChanged: if (!pressed) pane.request()
            background: Rectangle {
                x: scrub.leftPadding; y: scrub.topPadding + scrub.availableHeight / 2 - height / 2
                width: scrub.availableWidth; height: 4; radius: 2
                color: Tokens.raised
                Rectangle { width: scrub.visualPosition * parent.width; height: parent.height; radius: 2; color: Tokens.accent }
            }
            handle: Rectangle {
                x: scrub.leftPadding + scrub.visualPosition * (scrub.availableWidth - width)
                y: scrub.topPadding + scrub.availableHeight / 2 - height / 2
                width: 16; height: 16; radius: 8
                color: scrub.pressed ? Tokens.text : Tokens.accent
            }
        }
        IconButton { text: "▶"; enabled: pane.ctl.loaded; onClicked: pane.step(10) }
        IconButton { text: "⏩"; enabled: pane.ctl.loaded; onClicked: pane.step(60) }
    }

    // ---- Mode, crop actions --------------------------------------------------
    RowLayout {
        Layout.fillWidth: true
        spacing: 8

        Rectangle {
            implicitWidth: modes.implicitWidth + 8
            implicitHeight: 34
            radius: Tokens.radius
            color: Tokens.surface
            border.color: Tokens.border
            Row {
                id: modes
                anchors.centerIn: parent
                spacing: 4
                Repeater {
                    model: ["Source", "After filters"]
                    Rectangle {
                        id: modeItem
                        required property string modelData
                        required property int index
                        readonly property bool on: (index === 1) === pane.filtered
                        width: modeLabel.implicitWidth + 22
                        height: 26
                        radius: 6
                        color: on ? Tokens.raised : "transparent"
                        Text { id: modeLabel; anchors.centerIn: parent; text: modeItem.modelData; color: modeItem.on ? Tokens.text : Tokens.dim; font.pixelSize: 12; font.weight: Font.DemiBold }
                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: { pane.filtered = (modeItem.index === 1); pane.request() }
                        }
                    }
                }
            }
        }
        PrimaryButton { text: pane.ctl.busy ? "Working…" : "Auto-crop"; quiet: true; enabled: !pane.ctl.busy; onClicked: pane.ctl.autocrop() }
        PrimaryButton { text: "No crop"; quiet: true; enabled: !pane.ctl.busy && (pane.ctl.cropl + pane.ctl.cropr + pane.ctl.cropt + pane.ctl.cropb) > 0; onClicked: pane.ctl.setcrop(0, 0, 0, 0) }
        Item { Layout.fillWidth: true }
        Text {
            text: pane.filtered ? "Exact frame from the encode script" : "Drag the edges to adjust the crop"
            color: Tokens.dim
            font.pixelSize: 11
        }
    }
}
