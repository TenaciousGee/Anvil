// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Layouts
import com.anvil.app

// Title and one-line explanation at the top of each page.
ColumnLayout {
    property string title
    property string subtitle
    spacing: 4
    // Nested layouts fill spare height by default; a header never should.
    Layout.fillHeight: false
    Text { text: parent.title; color: Tokens.text; font.pixelSize: Tokens.hero; font.weight: Font.DemiBold }
    Text {
        Layout.fillWidth: true
        text: parent.subtitle
        color: Tokens.dim
        font.pixelSize: Tokens.body
        wrapMode: Text.WordWrap
        visible: text.length > 0
    }
}
