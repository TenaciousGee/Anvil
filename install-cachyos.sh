#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 The Anvil contributors (see AUTHORS)
# Anvil installer for CachyOS (and other Arch-based distributions).
#
# Installs build dependencies with pacman, builds the CLI and GUI in release
# mode, runs the tests, and can optionally install the binaries with a
# desktop menu entry.
#
# Usage: ./install-cachyos.sh [options]
#   --with-tools    Also install the encoding tools (x265, SVT-AV1, MKVToolNix,
#                   MediaInfo, VapourSynth, hdr10plus_tool, dovi_tool) from the repos,
#                   and Anvil's VapourSynth plugins (official PyPI builds, into
#                   ~/.local/share/anvil/vs)
#   --with-aur      Also install the GPU encoder for your graphics card from the
#                   AUR (NVEncC, QSVEncC or VCEEncC) using paru or yay (same as --gpu)
#   --gpu           Use GPU encoders: install yours without asking
#   --cpu-only      CPU encoding only: install no GPU encoders, hide GPU presets
#                   (without either, the script asks)
#   --install       Copy binaries to ~/.local/bin and add a menu entry
#   --no-gui        Build only the core and CLI (no Qt needed)
#   --skip-tests    Do not run the test suite
#   --yes           Do not ask for confirmation (passes --noconfirm)
#   --no-upgrade    Do not run 'pacman -Syu' before installing packages
#   --dry-run       Print what would be done without changing anything
#   -h, --help      Show this help

set -Eeuo pipefail

# ---- Options ----------------------------------------------------------------
WITH_TOOLS=0
WITH_AUR=0
DO_INSTALL=0
BUILD_GUI=1
RUN_TESTS=1
ASSUME_YES=0
DRY_RUN=0
DO_UPGRADE=1
GPU_CHOICE=""
MIN_RUST="1.85"

usage() { sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; }

for arg in "$@"; do
    case "$arg" in
        --with-tools) WITH_TOOLS=1 ;;
        --with-aur|--gpu) WITH_AUR=1; GPU_CHOICE=yes ;;
        --cpu-only) WITH_AUR=0; GPU_CHOICE=no ;;
        --install) DO_INSTALL=1 ;;
        --no-gui) BUILD_GUI=0 ;;
        --skip-tests) RUN_TESTS=0 ;;
        --no-upgrade) DO_UPGRADE=0 ;;
        --yes|-y) ASSUME_YES=1 ;;
        --dry-run) DRY_RUN=1 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown option: $arg" >&2; usage; exit 2 ;;
    esac
done

# ---- Output helpers -----------------------------------------------------------
if [[ -t 1 ]]; then
    BOLD=$'\e[1m'; DIM=$'\e[2m'; RED=$'\e[31m'; GREEN=$'\e[32m'; YELLOW=$'\e[33m'; BLUE=$'\e[34m'; RESET=$'\e[0m'
else
    BOLD=""; DIM=""; RED=""; GREEN=""; YELLOW=""; BLUE=""; RESET=""
fi
step() { printf '\n%s==>%s %s%s%s\n' "$BLUE" "$RESET" "$BOLD" "$*" "$RESET"; }
info() { printf '    %s\n' "$*"; }
ok()   { printf '    %s✔%s %s\n' "$GREEN" "$RESET" "$*"; }
warn() { printf '    %s!%s %s\n' "$YELLOW" "$RESET" "$*"; }
die()  { printf '\n%sError:%s %s\n' "$RED" "$RESET" "$*" >&2; exit 1; }

# Never stop silently: report the line and command of any unexpected failure.
on_error() {
    local rc=$1 line=$2 cmd=$3
    printf '\n%sError:%s command failed (exit %s) at line %s:\n    %s\n' "$RED" "$RESET" "$rc" "$line" "$cmd" >&2
    exit "$rc"
}
trap 'on_error $? $LINENO "$BASH_COMMAND"' ERR

# Run a command, or just print it in dry-run mode.
run() {
    if (( DRY_RUN )); then
        printf '    %s[dry-run]%s %s\n' "$DIM" "$RESET" "$*"
    else
        "$@" || {
            local rc=$?
            printf '\n%sError:%s this command failed (exit %s):\n    %s\n' "$RED" "$RESET" "$rc" "$*" >&2
            exit "$rc"
        }
    fi
}

PACMAN_FLAGS=(--needed)
(( ASSUME_YES )) && PACMAN_FLAGS+=(--noconfirm)

# ---- Sanity checks ------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

[[ -f Cargo.toml && -d crates/anvil-core ]] \
    || die "Run this script from the Anvil project folder (Cargo.toml not found next to it)."

# ANVIL_ALLOW_ROOT=1 permits root, e.g. inside a build container.
if [[ $EUID -eq 0 && "${ANVIL_ALLOW_ROOT:-0}" != "1" ]]; then
    die "Please run as your normal user, not root. The script uses sudo only for pacman."
fi

command -v pacman >/dev/null 2>&1 \
    || die "pacman not found. This script is for CachyOS and other Arch-based systems."

if [[ -r /etc/os-release ]]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    info "Detected ${PRETTY_NAME:-unknown distribution}"
fi

# version_ge A B: true when version A >= version B
version_ge() { [[ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" == "$2" ]]; }

pkg_installed() { pacman -Q "$1" >/dev/null 2>&1; }
pkg_in_repos()  { pacman -Si "$1" >/dev/null 2>&1; }

# Install packages from the repos, skipping any that do not exist so a
# renamed package never aborts the whole run.
MIRROR_HINT="This is usually an out-of-date mirror. On CachyOS run 'sudo cachyos-rate-mirrors' (on Arch, 'reflector'), then rerun this script."

# Arch only supports installing against an up-to-date system: installing from
# a stale package list asks mirrors for versions they have already deleted
# (HTTP 404). So refresh and upgrade once, before the first install.
SYNCED=0
ensure_synced() {
    (( SYNCED )) && return 0
    SYNCED=1
    if (( ! DO_UPGRADE )); then
        warn "Skipping 'pacman -Syu' (--no-upgrade); downloads may fail if your package lists are old."
        return 0
    fi
    info "Refreshing package lists and updating the system first (pacman -Syu)"
    if ! try_run sudo pacman -Syu "${PACMAN_FLAGS[@]}"; then
        die "System update failed. $MIRROR_HINT"
    fi
}

# Like run, but returns the exit status instead of stopping the script.
try_run() {
    if (( DRY_RUN )); then
        printf '    %s[dry-run]%s %s\n' "$DIM" "$RESET" "$*"
        return 0
    fi
    "$@"
}

# Install packages from the repos, skipping any that do not exist so a
# renamed package never aborts the whole run.
#   pacman_install [--optional] pkg...
# With --optional, a failed download is a warning: the batch is retried one
# package at a time and whatever cannot be installed is reported at the end.
pacman_install() {
    local optional=0
    if [[ "${1:-}" == "--optional" ]]; then optional=1; shift; fi
    local wanted=() missing=() failed=()
    for p in "$@"; do
        if pkg_installed "$p"; then
            continue
        elif pkg_in_repos "$p"; then
            wanted+=("$p")
        else
            missing+=("$p")
        fi
    done
    for p in "${missing[@]}"; do warn "Package '$p' not found in your repositories, skipping"; done
    if (( ! ${#wanted[@]} )); then
        ok "Already installed: $*"
        return 0
    fi
    ensure_synced
    info "Installing: ${wanted[*]}"
    if try_run sudo pacman -S "${PACMAN_FLAGS[@]}" "${wanted[@]}"; then
        return 0
    fi
    if (( ! optional )); then
        die "Could not install required packages: ${wanted[*]}. $MIRROR_HINT"
    fi
    warn "Batch install failed; trying each package on its own"
    for p in "${wanted[@]}"; do
        try_run sudo pacman -S "${PACMAN_FLAGS[@]}" "$p" || failed+=("$p")
    done
    if (( ${#failed[@]} )); then
        warn "Could not install: ${failed[*]}. Everything else is in place."
        warn "$MIRROR_HINT"
        OPTIONAL_FAILED+=("${failed[@]}")
    fi
}
OPTIONAL_FAILED=()

# ---- GPU encoders: ask once, up front ----------------------------------------------
# GPU encoders come from the AUR and can take a long time to build (NVIDIA's
# needs the 4 GB CUDA toolkit). Choosing CPU only skips them entirely and
# tells Anvil to hide the GPU presets (hardware_encoders = false).
if [[ -z "$GPU_CHOICE" ]]; then
    if [[ -t 0 ]] && (( ! ASSUME_YES )); then
        step "GPU encoders"
        info "Anvil can also encode on your graphics card (NVIDIA, Intel or AMD): much faster,"
        info "slightly larger files for the same quality. The encoders are built from the AUR,"
        info "which can take a long time (NVIDIA also needs the CUDA toolkit, about 4 GB)."
        info "Choosing no keeps Anvil to CPU encoding (x265, x264, SVT-AV1, rav1e); you can"
        info "change this later on the Tools page or by rerunning with --gpu."
        read -r -p "    Install GPU encoders now? [y/N] " answer || answer=""
        case "${answer,,}" in
            y|yes) GPU_CHOICE=yes; WITH_AUR=1 ;;
            *) GPU_CHOICE=no ;;
        esac
    fi
    # Not interactive (or --yes) and no --gpu/--cpu-only: install no GPU
    # encoders and leave the app's setting as it is.
fi

# Remember the choice for the app (keeps the Forge mode setting).
write_gpu_setting() {
    local on=$1 dir="${XDG_CONFIG_HOME:-$HOME/.config}/anvil" forge=false
    local file="$dir/settings.toml"
    if (( DRY_RUN )); then
        info "[dry-run] hardware_encoders = $on in $file"
        return
    fi
    mkdir -p "$dir"
    [[ -f "$file" ]] && grep -Eq '^forge *= *true' "$file" && forge=true
    printf '# Anvil settings\n# Forge mode: metalworking names in the interface.\nforge = %s\n# GPU encoders (NVIDIA, Intel, AMD, VAAPI). false = CPU encoding only.\nhardware_encoders = %s\n' \
        "$forge" "$on" > "$file"
}
case "$GPU_CHOICE" in
    yes) write_gpu_setting true; info "GPU encoders: on" ;;
    no) write_gpu_setting false; info "GPU encoders: off (CPU encoding only; GPU presets hidden)" ;;
    *) info "GPU encoders: not installing any (rerun with --gpu or --cpu-only to choose)" ;;
esac

# ---- 1. Build dependencies ------------------------------------------------------
step "Checking build dependencies"

BASE_PKGS=(base-devel git ffmpeg unzip)
GUI_PKGS=(qt6-base qt6-declarative qt6-wayland)

PKGS=("${BASE_PKGS[@]}")
(( BUILD_GUI )) && PKGS+=("${GUI_PKGS[@]}")

pacman_install "${PKGS[@]}"

# ---- 2. Rust toolchain ----------------------------------------------------------
step "Checking Rust (need $MIN_RUST or newer)"

rust_version() { rustc --version 2>/dev/null | awk '{print $2}' || true; }
# rustup installs a `rustc` proxy that fails until a default toolchain is
# chosen, so test that rustc actually runs rather than that it exists.
rust_works() { rustc --version >/dev/null 2>&1; }

if command -v rustup >/dev/null 2>&1; then
    if ! rust_works; then
        info "rustup is installed but no default toolchain is set; installing stable"
        run rustup default stable
    fi
elif rust_works; then
    : # system rust package; version checked below
else
    info "No Rust found, installing rustup"
    pacman_install rustup
    run rustup default stable
fi

if (( ! DRY_RUN )); then
    RV="$(rust_version)"
    [[ -n "$RV" ]] || die "rustc still does not run. Try 'rustup default stable' by hand, then open a new terminal and rerun this script."
    if ! version_ge "$RV" "$MIN_RUST"; then
        if command -v rustup >/dev/null 2>&1; then
            warn "Rust $RV is too old, updating stable toolchain"
            run rustup update stable
            RV="$(rust_version)"
            version_ge "$RV" "$MIN_RUST" || die "Rust $RV is still older than $MIN_RUST."
        else
            die "Rust $RV is older than $MIN_RUST. Update with 'sudo pacman -Syu rust', or switch to rustup: 'sudo pacman -S rustup' (this replaces the rust package)."
        fi
    fi
    ok "Rust $RV"
elif rust_works; then
    info "Found Rust $(rust_version)"
fi

if (( ! DRY_RUN )); then
    if ! cargo --version >/dev/null 2>&1 && [[ -x "$HOME/.cargo/bin/cargo" ]]; then
        export PATH="$HOME/.cargo/bin:$PATH"
    fi
    cargo --version >/dev/null 2>&1 \
        || die "cargo does not run. With rustup, try 'rustup default stable'; with the rust package, 'sudo pacman -S rust'. Then open a new terminal and rerun."
    ok "$(cargo --version)"
fi

# ---- 3. Qt checks -----------------------------------------------------------------
if (( BUILD_GUI )); then
    step "Checking Qt 6"
    QMAKE_BIN="$(command -v qmake6 || command -v qmake || true)"
    if [[ -z "$QMAKE_BIN" ]] && (( ! DRY_RUN )); then
        die "qmake6 not found even though qt6-base is installed. Check 'pacman -Ql qt6-base | grep qmake'."
    fi
    if [[ -n "$QMAKE_BIN" ]]; then
        export QMAKE="$QMAKE_BIN"
        QT_VER="$("$QMAKE_BIN" -query QT_VERSION 2>/dev/null || echo unknown)"
        ok "Using $QMAKE_BIN (Qt $QT_VER)"
        if pkg_installed qt6-base && pkg_installed qt6-declarative; then
            B="$(pacman -Q qt6-base | awk '{print $2}')"
            D="$(pacman -Q qt6-declarative | awk '{print $2}')"
            [[ "${B%%-*}" == "${D%%-*}" ]] \
                || warn "qt6-base ($B) and qt6-declarative ($D) differ; run 'sudo pacman -Syu' if the build fails to link."
        fi
    fi
fi

# ---- 4. Optional encoding tools -------------------------------------------------
if (( WITH_TOOLS )); then
    step "Installing encoding tools from the repositories"
    # Required for encoding.
    pacman_install x265 mkvtoolnix-cli vapoursynth
    # Nice to have: a failure here never stops the build.
    pacman_install --optional svt-av1 x264 rav1e mediainfo hdr10plus-tool dovi-tool tesseract tesseract-data-eng
    # vainfo, for the VAAPI encoder check (GPU encoding only).
    [[ "$GPU_CHOICE" == yes ]] && pacman_install --optional libva-utils

    # VapourSynth R80 and later (CachyOS ships R81) no longer load plugins
    # from /usr/lib/vapoursynth, and the AUR plugin packages fail to build
    # against it. Anvil installs each plugin's official PyPI build into its
    # own folder instead (~/.local/share/anvil/vs); nothing system-wide.
    step "Installing Anvil's VapourSynth plugins (source, deinterlace, denoise, deband, sharpen, grain)"
    if (( DRY_RUN )); then
        info "[dry-run] python3 packaging/scripts/install-vs-plugins.py"
    elif python3 -I packaging/scripts/install-vs-plugins.py 2>&1 | sed 's/^/    /'; [[ ${PIPESTATUS[0]} -eq 0 ]]; then
        ok "VapourSynth plugins ready"
    else
        warn "Some VapourSynth plugins could not be installed (see above). The Filters page lists what is missing; encodes without those filters still work."
    fi
fi

if (( WITH_AUR )); then
    step "Installing AUR extras"
    AUR_HELPER="$(command -v paru || command -v yay || true)"
    if [[ -z "$AUR_HELPER" ]]; then
        warn "No AUR helper (paru or yay) found. Install one, e.g. 'sudo pacman -S paru' on CachyOS, then rerun with --with-aur."
    else
        info "Using $(basename "$AUR_HELPER")"
        AUR_FLAGS=(--needed)
        (( ASSUME_YES )) && AUR_FLAGS+=(--noconfirm)
        # VapourSynth plugins come from PyPI (see --with-tools), not the AUR:
        # the AUR plugin packages do not build against VapourSynth R80+.
        # Each group lists alternative package names; the first one found is used.
        AUR_GROUPS=()
        # The GPU encoder for each graphics card found (rigaya's encoders),
        # by PCI vendor id: 10de NVIDIA, 8086 Intel, 1002 AMD. (Matching
        # names was wrong: "ati" is inside "Corporation".)
        GPUS="$(lspci -nn 2>/dev/null | grep -Ei 'vga|3d controller|display' || true)"
        if [[ -n "$GPUS" ]]; then
            info "Graphics found:"
            sed 's/^/      /' <<<"$GPUS"
        fi
        if grep -q '\[10de:' <<<"$GPUS"; then
            # NVEnc builds with NVIDIA's CUDA compiler (nvcc), from the cuda
            # package (a large download, about 4 GB). It installs to
            # /opt/cuda/bin, which is only on PATH after logging in again.
            if ! command -v nvcc >/dev/null 2>&1 && [[ ! -x /opt/cuda/bin/nvcc ]]; then
                info "NVEnc needs NVIDIA's CUDA compiler: installing the cuda package (about 4 GB)"
                pacman_install --optional cuda
            fi
            if [[ -x /opt/cuda/bin/nvcc ]]; then
                export PATH="/opt/cuda/bin:$PATH"
                [[ -d /opt/cuda/lib64 ]] && export LD_LIBRARY_PATH="/opt/cuda/lib64${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
            fi
            if command -v nvcc >/dev/null 2>&1; then
                AUR_GROUPS+=("nvenc nvenc-git")
            else
                warn "No CUDA compiler (nvcc), so NVEnc cannot be built. Use a CPU preset, or install cuda (sudo pacman -S cuda) and rerun with --with-aur."
            fi
        fi
        grep -q '\[8086:' <<<"$GPUS" && AUR_GROUPS+=("qsvenc qsvenc-git")
        grep -q '\[1002:' <<<"$GPUS" && AUR_GROUPS+=("vceenc vceenc-git")
        # An encoder for graphics this machine does not have (for example
        # VCEEncC on a Ryzen without integrated graphics) cannot work: say so.
        for pair in "nvenc:10de:NVIDIA" "nvenc-git:10de:NVIDIA" "qsvenc:8086:Intel" "qsvenc-git:8086:Intel" "vceenc:1002:AMD" "vceenc-git:1002:AMD"; do
            IFS=: read -r pkg vendor brand <<<"$pair"
            if pkg_installed "$pkg" && ! grep -q "\[$vendor:" <<<"$GPUS"; then
                warn "$pkg is installed but this machine has no $brand graphics, so Anvil will not use it. Remove it with: paru -R $pkg"
            fi
        done
        for group in "${AUR_GROUPS[@]}"; do
            found=""
            for name in $group; do
                if pkg_installed "$name"; then found="installed:$name"; break; fi
                if "$AUR_HELPER" -Si "$name" >/dev/null 2>&1; then found="$name"; break; fi
            done
        # An AUR build goes stale when a library it links to is updated
        # (for example libhdr10plus-rs.so.2 -> .so.3): it is installed but
        # will not start. Rebuild any GPU encoder in that state.
        for pair in "nvencc:nvenc" "qsvencc:qsvenc" "vceencc:vceenc"; do
            IFS=: read -r bin pkg <<<"$pair"
            command -v "$bin" >/dev/null 2>&1 || continue
            if "$bin" --version 2>&1 | grep -q "error while loading shared libraries"; then
                warn "$bin is installed but cannot start (a library it was built against has changed); rebuilding $pkg"
                pkg_installed "$pkg-git" && pkg="$pkg-git"
                # Not --needed: that would skip a package that is up to date.
                REBUILD_FLAGS=(--rebuild)
                (( ASSUME_YES )) && REBUILD_FLAGS+=(--noconfirm)
                try_run "$AUR_HELPER" -S "${REBUILD_FLAGS[@]}" "$pkg" || warn "Could not rebuild $pkg; carrying on"
            fi
        done
            case "$found" in
                installed:*) ok "Already installed: ${found#installed:}" ;;
                "") warn "None of [$group] found in the repos or AUR, skipping" ;;
                # try_run, not run: a package that fails to build must not
                # stop the whole install.
                *) info "Installing $found"; try_run "$AUR_HELPER" -S "${AUR_FLAGS[@]}" "$found" || warn "Could not install $found; carrying on (the Tools page shows which encoders work)" ;;
            esac
        done
    fi
fi

# ---- 5. Build ----------------------------------------------------------------------
step "Building Anvil (release)"
run cargo build --release -p anvil-core -p anvil-cli
if (( BUILD_GUI )); then
    info "Building the GUI. The first build compiles the Qt bindings and takes a few minutes."
    run cargo build --release -p anvil-app
fi

# ---- 6. Tests ------------------------------------------------------------------------
if (( RUN_TESTS )); then
    step "Running tests"
    run cargo test --release -p anvil-core -p anvil-cli
fi

# ---- 7. Install ----------------------------------------------------------------------
BIN_DIR="$HOME/.local/bin"
if (( DO_INSTALL )); then
    step "Installing to $BIN_DIR"
    run mkdir -p "$BIN_DIR"
    # Anvil was called Reelforge: remove the old programs and menu entry.
    for old in "$BIN_DIR/reelforge" "$BIN_DIR/reelforge-gui" "$HOME/.local/share/applications/reelforge.desktop"; do
        if [[ -e "$old" ]]; then
            run rm -f "$old"
            info "Removed the old $(basename "$old") (now Anvil)"
        fi
    done
    run install -m 755 target/release/anvil "$BIN_DIR/anvil"
    if (( BUILD_GUI )); then
        run install -m 755 target/release/anvil-gui "$BIN_DIR/anvil-gui"

        APP_DIR="$HOME/.local/share/applications"
        DESKTOP_FILE="$APP_DIR/io.github.anvil_video.Anvil.desktop"
        # The menu entry is named after the app id so the window finds its icon.
        [[ -e "$APP_DIR/anvil.desktop" ]] && run rm -f "$APP_DIR/anvil.desktop"
        ICON_ROOT="$HOME/.local/share/icons/hicolor"
        for s in 16 32 48 64 128 256 512; do
            run install -Dm644 "packaging/icons/anvil-$s.png" "$ICON_ROOT/${s}x$s/apps/io.github.anvil_video.Anvil.png"
        done
        command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$ICON_ROOT" 2>/dev/null || true
        run mkdir -p "$APP_DIR"
        if (( DRY_RUN )); then
            info "[dry-run] write $DESKTOP_FILE"
        else
            cat > "$DESKTOP_FILE" <<EOF
[Desktop Entry]
Type=Application
Name=Anvil
GenericName=Video Encoder
Comment=Encode DVD, Blu-ray and UHD rips with HDR and lossless audio preserved
Exec=$BIN_DIR/anvil-gui
Icon=io.github.anvil_video.Anvil
StartupWMClass=anvil-gui
Terminal=false
Categories=AudioVideo;Video;AudioVideoEditing;
Keywords=encode;x265;av1;hdr;dolby vision;blu-ray;dvd;
EOF
            if command -v update-desktop-database >/dev/null 2>&1; then
                update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
            fi
        fi
        ok "Menu entry added"
    fi
    case ":$PATH:" in
        *":$BIN_DIR:"*) ;;
        *)
            warn "$BIN_DIR is not on your PATH yet."
            info "fish (the CachyOS default):  fish_add_path ~/.local/bin"
            info "bash:  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc"
            info "zsh:   echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc"
            ;;
    esac
fi

# ---- Summary ---------------------------------------------------------------------------
step "Done"
if (( DO_INSTALL )); then
    info "Run the GUI:  anvil-gui   (or find Anvil in your app menu)"
    info "Run the CLI:  anvil --help"
else
    (( BUILD_GUI )) && info "Run the GUI:  ./target/release/anvil-gui"
    info "Run the CLI:  ./target/release/anvil --help"
    info "Rerun with --install to put them on your PATH with a menu entry."
fi
if (( ${#OPTIONAL_FAILED[@]} )); then
    warn "Optional packages not installed: ${OPTIONAL_FAILED[*]}. Anvil works without them; retry later with --with-tools."
fi
if (( ! WITH_TOOLS )); then
    info "Encoding tools are not installed yet; rerun with --with-tools (and --with-aur for your GPU's encoder)."
fi
