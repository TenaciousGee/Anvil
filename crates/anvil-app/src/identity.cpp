// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
// Window icon and desktop-file link for the running app. The icon is
// compiled in (qrc), so it also shows when Anvil is run from the build
// folder; the desktop file name lets Wayland compositors match the window
// to the installed menu entry and its icon.
#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>

extern "C" void anvil_set_identity(const char *version)
{
    QGuiApplication::setWindowIcon(QIcon(QStringLiteral(":/qt/qml/com/anvil/app/assets/anvil.png")));
    QGuiApplication::setDesktopFileName(QStringLiteral("io.github.anvil_video.Anvil"));
    QGuiApplication::setApplicationDisplayName(QStringLiteral("Anvil"));
    QGuiApplication::setApplicationVersion(QString::fromUtf8(version));
}
