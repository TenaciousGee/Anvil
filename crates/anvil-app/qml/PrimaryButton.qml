// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Controls
import com.anvil.app

// Filled accent button; `quiet` gives the outlined secondary style.
Button {
    id: pb
    property bool quiet: false
    property bool danger: false

    hoverEnabled: true
    leftPadding: 16
    rightPadding: 16

    contentItem: Text {
        text: pb.text
        color: pb.quiet ? (pb.danger ? "#FF8A8A" : Tokens.text) : Tokens.ink
        font.pixelSize: Tokens.body
        font.weight: Font.DemiBold
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        opacity: pb.enabled ? 1 : 0.45
    }
    background: Rectangle {
        implicitHeight: 36
        implicitWidth: 88
        radius: Tokens.radius
        color: pb.quiet
            ? (pb.hovered && pb.enabled ? Tokens.raised : Tokens.surface)
            : (pb.down ? Qt.darker(Tokens.accent, 1.15) : (pb.hovered ? Qt.lighter(Tokens.accent, 1.08) : Tokens.accent))
        border.color: pb.quiet ? Tokens.border : "transparent"
        opacity: pb.enabled ? 1 : 0.45
        Behavior on color { ColorAnimation { duration: 120 } }
    }
}
