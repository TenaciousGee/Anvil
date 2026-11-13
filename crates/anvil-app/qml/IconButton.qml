// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Controls
import com.anvil.app

// Small square button for row actions (reorder, remove).
Button {
    id: ib
    hoverEnabled: true
    implicitWidth: 28
    implicitHeight: 28
    contentItem: Text {
        text: ib.text
        color: ib.enabled ? Tokens.text : Tokens.border
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        font.pixelSize: 12
    }
    background: Rectangle {
        radius: 6
        color: ib.hovered && ib.enabled ? Tokens.raised : "transparent"
        border.color: Tokens.border
    }
}
