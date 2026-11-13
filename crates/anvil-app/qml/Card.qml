// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Layouts
import com.anvil.app

// Surface with a title; children go into a column under the title.
Rectangle {
    id: card
    property string title
    property string trailing
    default property alias content: body.data

    color: Tokens.surface
    radius: Tokens.radius
    border.color: Tokens.border
    implicitHeight: column.implicitHeight + 32

    ColumnLayout {
        id: column
        anchors.fill: parent
        anchors.margins: 16
        spacing: 12

        RowLayout {
            Layout.fillWidth: true
            visible: card.title.length > 0
            Text {
                text: card.title
                color: Tokens.dim
                font.pixelSize: 11
                font.weight: Font.DemiBold
                font.capitalization: Font.AllUppercase
                font.letterSpacing: 1.2
                Layout.fillWidth: true
            }
            Text {
                text: card.trailing
                color: Tokens.dim
                font.pixelSize: 12
                visible: text.length > 0
            }
        }

        ColumnLayout {
            id: body
            Layout.fillWidth: true
            spacing: 8
        }
    }
}
