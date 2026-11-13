// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Controls
import com.anvil.app

// Themed single-line text field. `committed(text)` fires on Enter or when
// focus leaves with a changed value.
TextField {
    id: f
    property string committedText
    signal committed(string text)

    text: committedText
    color: Tokens.text
    placeholderTextColor: Tokens.dim
    // Defaults shown as placeholders read as hints, not values.
    font.italic: text.length === 0 && placeholderText.length > 0
    selectByMouse: true
    font.pixelSize: 12
    leftPadding: 8
    rightPadding: 8
    background: Rectangle {
        implicitHeight: 32
        implicitWidth: 80
        radius: 6
        color: Tokens.bg
        border.color: f.activeFocus ? Tokens.accent : Tokens.border
    }
    onAccepted: if (text !== committedText) f.committed(text)
    onActiveFocusChanged: if (!activeFocus && text !== committedText) f.committed(text)
}
