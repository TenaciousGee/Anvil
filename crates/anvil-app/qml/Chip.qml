// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import com.anvil.app

// A small status badge: "HDR10", "Dolby Vision 8.1", "TrueHD".
Rectangle {
    id: chip
    property alias text: label.text
    property color tone: Tokens.accent

    implicitWidth: label.implicitWidth + 20
    implicitHeight: 26
    radius: 13
    color: Qt.rgba(tone.r, tone.g, tone.b, 0.14)
    border.color: Qt.rgba(tone.r, tone.g, tone.b, 0.45)

    Text {
        id: label
        anchors.centerIn: parent
        color: chip.tone
        font.pixelSize: 12
        font.weight: Font.DemiBold
    }
}
