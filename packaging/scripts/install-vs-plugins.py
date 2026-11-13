#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 The Anvil contributors (see AUTHORS)
"""Install the VapourSynth plugins Anvil uses, from their official PyPI builds.

usage: install-vs-plugins.py [--list] [--force]

VapourSynth R80 and later no longer load plugins from /usr/lib/vapoursynth
or a user plugin folder, and many distribution plugin packages fail to build
against it. Anvil therefore keeps its own plugin set: each plugin's
official wheel from PyPI (the same builds `pip install` would use) is
checked against its SHA-256 and unpacked, with the libraries it bundles,
into ~/.local/share/anvil/vs. Anvil points VapourSynth at
~/.local/share/anvil/vs/vapoursynth/plugins with VAPOURSYNTH_EXTRA_PLUGIN_PATH.
Nothing is installed system-wide and pip is not needed.

Exit status: 0 when every plugin loads, 1 otherwise.
"""
import hashlib
import io
import json
import os
import pathlib
import platform
import subprocess
import sys
import urllib.request
import zipfile

# (PyPI name, version, VapourSynth namespace, what Anvil uses it for)
PLUGINS = [
    ("vapoursynth-bestsource", "23", "bs", "source filter (default)"),
    ("vapoursynth-lsmas", "1310.0.0.0", "lsmas", "source filter (L-SMASH)"),
    ("vapoursynth-d2vsource", "1.4", "d2v", "DVD D2V projects"),
    ("vapoursynth-vivtc", "2.0", "vivtc", "inverse telecine"),
    ("vapoursynth-bwdif", "5.1", "bwdif", "deinterlacing"),
    ("vapoursynth-vszip", "24.0.0", "vszip", "denoise and deband"),
    ("vapoursynth-cas", "3.0", "cas", "sharpen"),
    ("vsnoise", "1.3", "noise", "film grain"),
    ("vapoursynth-znedi3", "3.3", "znedi3", "QTGMC (with havsfunc)"),
    ("vapoursynth-mvtools", "29", "mv", "QTGMC (with havsfunc)"),
]

home = pathlib.Path.home()
data_home = pathlib.Path(os.environ.get("XDG_DATA_HOME") or home / ".local/share")
ROOT = data_home / "anvil" / "vs"
PLUGIN_DIR = ROOT / "vapoursynth" / "plugins"


def vs_info():
    """(major version, set of loaded namespaces) with our folder added."""
    env = dict(os.environ, VAPOURSYNTH_EXTRA_PLUGIN_PATH=str(PLUGIN_DIR))
    code = ("import vapoursynth as vs\n"
            "print(vs.core.core_version.release_major)\n"
            "print(' '.join(p.namespace for p in vs.core.plugins()))")
    try:
        out = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True,
                             timeout=120, env=env)
        lines = out.stdout.splitlines()
        return int(lines[0]), set(lines[1].split()) if len(lines) > 1 else set()
    except Exception:
        return None, set()


def wheel_for(name, version):
    """(filename, url, sha256) of the Linux wheel for this machine."""
    url = f"https://pypi.org/pypi/{name}/{version}/json"
    meta = json.load(urllib.request.urlopen(url, timeout=60))
    arch = platform.machine()

    def score(fn):
        if "none-any" in fn:
            return 1
        if arch in fn and "manylinux" in fn:
            return 3
        return -1  # musllinux, other CPUs, macOS, Windows

    wheels = [f for f in meta["urls"] if f["filename"].endswith(".whl")]
    best = max(wheels, key=lambda f: score(f["filename"]), default=None)
    if not best or score(best["filename"]) <= 0:
        raise RuntimeError(f"no Linux {arch} build")
    return best["filename"], best["url"], best["digests"]["sha256"]


def install(name, version):
    fn, url, sha = wheel_for(name, version)
    blob = urllib.request.urlopen(url, timeout=300).read()
    if hashlib.sha256(blob).hexdigest() != sha:
        raise RuntimeError(f"{fn}: checksum does not match PyPI's")
    with zipfile.ZipFile(io.BytesIO(blob)) as z:
        for member in z.infolist():
            top = member.filename.split("/", 1)[0]
            # The plugin (vapoursynth/plugins/...) and the libraries it
            # bundles (<name>.libs/, found through a relative RPATH, so the
            # layout is kept). Python code and metadata are not needed.
            if not (member.filename.startswith("vapoursynth/plugins/") or top.endswith(".libs")):
                continue
            target = ROOT / member.filename
            if member.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(z.read(member))
            if target.suffix == ".so" or ".so." in target.name:
                target.chmod(0o755)
    return fn


def main():
    args = set(sys.argv[1:])
    if "--list" in args:
        for name, version, ns, use in PLUGINS:
            print(f"{ns:<8} {name} {version}: {use}")
        return 0
    major, loaded = vs_info()
    if major is None:
        print("VapourSynth's Python module did not load; install vapoursynth first.")
        return 1
    if major < 80:
        print(f"VapourSynth R{major} is older than R80 and does not read VAPOURSYNTH_EXTRA_PLUGIN_PATH.")
        print("Install the plugins with your package manager instead (see the Filters page).")
        return 1
    print(f"VapourSynth R{major}; plugins go to {PLUGIN_DIR}")
    for name, version, ns, _use in PLUGINS:
        if ns in loaded and "--force" not in args:
            print(f"  ok   {ns:<8} already available")
            continue
        try:
            print(f"  new  {ns:<8} {install(name, version)}")
        except Exception as e:  # network, checksum, no build for this CPU
            print(f"  FAIL {ns:<8} {name} {version}: {e}")
    _, loaded = vs_info()
    missing = [ns for _, _, ns, _ in PLUGINS if ns not in loaded]
    if missing:
        print("Not loading: " + ", ".join(missing))
        return 1
    print("Every plugin loads.")
    return 0


sys.exit(main())
