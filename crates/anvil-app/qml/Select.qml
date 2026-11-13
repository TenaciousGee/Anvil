// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Controls
import com.anvil.app

// Themed drop-down. `options` is a list of [value, label] pairs; `value`
// is the selected value and `picked(value)` fires when the user chooses.
ComboBox {
    id: sel
    property var options: []
    property string value
    signal picked(string value)

    model: options.map(o => o[1])
    currentIndex: Math.max(0, options.findIndex(o => o[0] === value))
    onActivated: (i) => sel.picked(options[i][0])
    implicitHeight: 32
    font.pixelSize: 12

    contentItem: Text {
        leftPadding: 10
        rightPadding: 24
        text: sel.displayText
        color: Tokens.text
        font.pixelSize: 12
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }
    indicator: Text {
        x: sel.width - width - 10
        anchors.verticalCenter: parent.verticalCenter
        text: "▾"
        color: Tokens.dim
        font.pixelSize: 12
    }
    background: Rectangle {
        radius: 6
        color: Tokens.bg
        border.color: sel.activeFocus ? Tokens.accent : Tokens.border
    }
    popup.background: Rectangle { color: Tokens.raised; border.color: Tokens.border; radius: 6 }
    delegate: ItemDelegate {
        required property int index
        required property var modelData
        width: sel.width
        highlighted: sel.highlightedIndex === index
        contentItem: Text { text: modelData; color: Tokens.text; font.pixelSize: 12; elide: Text.ElideRight }
        background: Rectangle { color: highlighted ? Tokens.tint : "transparent" }
    }
}
