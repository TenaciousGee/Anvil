#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 The Anvil contributors (see AUTHORS)
# Build Anvil-x86_64.AppImage: Anvil (GUI and CLI) with its Qt libraries
# and QML modules. The encoding tools (FFmpeg, x265, MKVToolNix,
# VapourSynth...) come from your system, as VapourSynth's Python plugins
# cannot be bundled reliably; the Tools page lists anything missing. For a
# build with every tool inside, use the Flatpak.
#
# Needs: Rust, Qt 6 (qmake6), curl. Works without FUSE (extract-and-run).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
OUT=${OUT:-$ROOT/dist}
TOOLS=${TOOLS:-$ROOT/target/appimage-tools}
APPDIR=$ROOT/target/AppDir
ID=io.github.anvil_video.Anvil
export APPIMAGE_EXTRACT_AND_RUN=1

mkdir -p "$OUT" "$TOOLS"
fetch() {
    [ -x "$TOOLS/$1" ] && return
    echo "==> Downloading $1"
    curl -fsSL -o "$TOOLS/$1" "$2"
    chmod +x "$TOOLS/$1"
}
fetch linuxdeploy-x86_64.AppImage https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
fetch linuxdeploy-plugin-qt-x86_64.AppImage https://github.com/linuxdeploy/linuxdeploy-plugin-qt/releases/download/continuous/linuxdeploy-plugin-qt-x86_64.AppImage

echo "==> Building release binaries"
(cd "$ROOT" && cargo build --release -p anvil-cli -p anvil-app)

rm -rf "$APPDIR"
install -Dm755 "$ROOT/target/release/anvil" "$APPDIR/usr/bin/anvil"
install -Dm755 "$ROOT/target/release/anvil-gui" "$APPDIR/usr/bin/anvil-gui"
install -Dm644 "$ROOT/packaging/$ID.desktop" "$APPDIR/usr/share/applications/$ID.desktop"
install -Dm644 "$ROOT/packaging/$ID.metainfo.xml" "$APPDIR/usr/share/metainfo/$ID.appdata.xml"
# Licence, authors and third-party notices (the Qt LGPL notice included).
install -Dm644 -t "$APPDIR/usr/share/doc/anvil" "$ROOT/LICENSE" "$ROOT/AUTHORS" "$ROOT/THIRD-PARTY.md"
for s in 16 32 48 64 128 256 512; do
    install -Dm644 "$ROOT/packaging/icons/anvil-$s.png" "$APPDIR/usr/share/icons/hicolor/${s}x$s/apps/$ID.png"
done

echo "==> Bundling Qt"
export QMAKE=${QMAKE:-$(command -v qmake6 || command -v qmake)}
# qmlimportscanner reads our QML to find the Qt Quick modules it imports.
export QML_SOURCES_PATHS="$ROOT/crates/anvil-app/qml"
# Wayland as well as X11, when this Qt has the Wayland platform plugin.
# Offscreen too, for headless runs (--screenshot, tests).
EXTRA_PLATFORM_PLUGINS="libqoffscreen.so"
if [ -e "$("$QMAKE" -query QT_INSTALL_PLUGINS)/platforms/libqwayland-generic.so" ]; then
    EXTRA_PLATFORM_PLUGINS="$EXTRA_PLATFORM_PLUGINS;libqwayland-generic.so;libqwayland-egl.so"
else
    echo "    (no Qt Wayland plugin here: the AppImage will use X11/XWayland)"
fi
export EXTRA_PLATFORM_PLUGINS
export LDAI_OUTPUT="$OUT/Anvil-x86_64.AppImage"
export OUTPUT="$LDAI_OUTPUT"
cd "$TOOLS"
./linuxdeploy-x86_64.AppImage --appdir "$APPDIR" \
    --executable "$APPDIR/usr/bin/anvil-gui" \
    --executable "$APPDIR/usr/bin/anvil" \
    --desktop-file "$APPDIR/usr/share/applications/$ID.desktop" \
    --icon-file "$APPDIR/usr/share/icons/hicolor/256x256/apps/$ID.png" \
    --plugin qt \
    --custom-apprun "$ROOT/packaging/appimage/AppRun" \
    --output appimage
echo "==> Built $LDAI_OUTPUT"
