// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import com.anvil.app

// Navigation rail entry: glyph over a short label.
Item {
    id: nav
    property string glyph
    property string label
    property bool active: false
    signal clicked()

    width: 72
    height: 64

    Rectangle {
        anchors.centerIn: parent
        width: 52
        height: 52
        radius: Tokens.radius
        color: nav.active ? Tokens.tint : (mouse.containsMouse ? Tokens.surface : "transparent")
        Behavior on color { ColorAnimation { duration: 150 } }
    }

    Column {
        anchors.centerIn: parent
        spacing: 2
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: nav.glyph
            color: nav.active ? Tokens.accent : Tokens.dim
            font.pixelSize: 18
        }
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: nav.label
            color: nav.active ? Tokens.text : Tokens.dim
            font.pixelSize: 10
        }
    }

    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: nav.clicked()
    }
}
