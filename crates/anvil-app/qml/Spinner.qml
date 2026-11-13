// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import com.anvil.app

// Small accent arc that spins while work runs in the background.
Item {
    id: spinner
    property bool running: true
    implicitWidth: 18
    implicitHeight: 18
    visible: running

    Canvas {
        id: arc
        anchors.fill: parent
        onPaint: {
            const ctx = getContext("2d")
            ctx.reset()
            ctx.lineWidth = 2.5
            ctx.lineCap = "round"
            ctx.strokeStyle = Tokens.accent
            ctx.beginPath()
            ctx.arc(width / 2, height / 2, width / 2 - 2, 0, Math.PI * 1.4)
            ctx.stroke()
        }
        RotationAnimation on rotation {
            running: spinner.running
            from: 0; to: 360
            duration: 900
            loops: Animation.Infinite
        }
    }
}
