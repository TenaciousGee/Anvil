// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Layouts
import com.anvil.app

// Error banner: impossible to miss, dismissable.
Rectangle {
    id: banner
    property string text
    signal dismissed()

    visible: text.length > 0
    implicitHeight: visible ? row.implicitHeight + 24 : 0
    radius: Tokens.radius
    color: "#2A1517"
    border.color: "#7A2E33"

    RowLayout {
        id: row
        anchors.fill: parent
        anchors.margins: 12
        anchors.leftMargin: 16
        spacing: 12
        Text { text: "⚠"; color: "#FF8A8A"; font.pixelSize: 16 }
        Text {
            Layout.fillWidth: true
            text: banner.text
            color: Tokens.text
            wrapMode: Text.WordWrap
            font.pixelSize: Tokens.body
        }
        IconButton { text: "✕"; onClicked: banner.dismissed() }
    }
}
