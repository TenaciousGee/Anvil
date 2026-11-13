#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 The Anvil contributors (see AUTHORS)
"""Check every git source's tag in the Flatpak manifest and pin its commit.

usage: pin-commits.py [MANIFEST]   (default: io.github.anvil_video.Anvil.yml)

Flathub wants each git source pinned to a commit as well as a tag. For each
`tag:` this asks the remote for the commit (following annotated tags) and
writes or updates the `commit:` line under it. Unreachable hosts and
missing tags are reported and left alone.
"""
import pathlib
import re
import subprocess
import sys

path = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else pathlib.Path(__file__).with_name("io.github.anvil_video.Anvil.yml"))
lines = path.read_text().splitlines()
out, url, problems = [], None, 0
i = 0
while i < len(lines):
    line = lines[i]
    m = re.match(r"(\s*)url: (\S+\.git)$", line)
    if m:
        url = m.group(2)
    t = re.match(r"(\s*)tag: \"?([^\"\s]+)\"?$", line)
    out.append(line)
    if t and url:
        indent, tag = t.group(1), t.group(2)
        # Drop an existing commit line right after the tag.
        if i + 1 < len(lines) and lines[i + 1].strip().startswith("commit:"):
            i += 1
        try:
            refs = subprocess.run(["git", "ls-remote", url, f"refs/tags/{tag}", f"refs/tags/{tag}^{{}}"],
                                  capture_output=True, text=True, timeout=60).stdout.split("\n")
            refs = [r.split("\t") for r in refs if r]
            commit = next((c for c, r in refs if r.endswith("^{}")), None) or next((c for c, _ in refs), None)
        except subprocess.TimeoutExpired:
            commit = None
        if commit:
            out.append(f"{indent}commit: {commit}")
            print(f"ok   {url} {tag} {commit[:12]}")
        else:
            problems += 1
            print(f"FAIL {url} {tag}: tag not found or host unreachable")
            if i < len(lines) and lines[i].strip().startswith("commit:"):
                out.append(lines[i])
        url = None
    i += 1
path.write_text("\n".join(out) + "\n")
sys.exit(1 if problems else 0)
