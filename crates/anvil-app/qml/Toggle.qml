// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Controls
import com.anvil.app

// Themed check box with a label.
CheckBox {
    id: t
    font.pixelSize: 12
    spacing: 6
    indicator: Rectangle {
        implicitWidth: 16; implicitHeight: 16
        x: t.leftPadding
        y: (t.height - height) / 2
        radius: 4
        color: t.checked ? Tokens.accent : Tokens.bg
        border.color: t.checked ? Tokens.accent : Tokens.border
        Text { anchors.centerIn: parent; text: "✓"; color: Tokens.ink; font.pixelSize: 11; font.weight: Font.Black; visible: t.checked }
    }
    contentItem: Text {
        leftPadding: t.indicator.width + t.spacing
        text: t.text
        color: t.enabled ? Tokens.text : Tokens.dim
        font.pixelSize: 12
        verticalAlignment: Text.AlignVCenter
    }
}
