# Third-party software

Anvil itself is licensed under the [GNU General Public License, version 3 or later](LICENSE) (GPL-3.0-or-later).

Anvil does not link to any of the encoding tools below. It runs them as separate programs and passes files between them, so each keeps its own licence. What is shipped with Anvil depends on how you installed it.

## Built into the Anvil programs

The Rust crates compiled into `anvil` and `anvil-gui` are all under permissive licences (MIT, Apache-2.0, BSD, ISC, Unicode-3.0, Zlib and similar), all compatible with the GPL. `cargo tree` and [`Cargo.lock`](Cargo.lock) list them with their versions; each crate's source is on [crates.io](https://crates.io).

| Component | Licence | Source |
| --- | --- | --- |
| CXX-Qt (Rust and Qt bindings) | MIT or Apache-2.0 | https://github.com/KDAB/cxx-qt |

## Shipped in the AppImage

| Component | Licence | Source |
| --- | --- | --- |
| Qt 6 (Core, Gui, Qml, Quick, Quick Controls 2, Wayland and X11 platform plugins) | LGPL-3.0 | https://download.qt.io/official_releases/qt/ |

Qt is linked dynamically and its libraries sit unmodified in `usr/lib` inside the AppImage. Under the LGPL you may replace them with your own build of the same Qt version: extract the AppImage with `--appimage-extract`, swap the libraries in `squashfs-root/usr/lib` and start `squashfs-root/AppRun`. The text of the LGPL is at https://www.gnu.org/licenses/lgpl-3.0.html.

The encoding tools are not in the AppImage; it uses the ones installed on your system.

## Shipped in the Flatpak

The Flatpak builds these from source (the exact versions and commits are in [`packaging/flatpak/io.github.anvil_video.Anvil.yml`](packaging/flatpak/io.github.anvil_video.Anvil.yml)) and runs on the KDE Platform runtime, which provides Qt under the LGPL-3.0.

| Component | Licence | Source |
| --- | --- | --- |
| FFmpeg (built with `--enable-gpl --enable-version3`) | GPL-3.0-or-later | https://ffmpeg.org/download.html |
| dav1d | BSD-2-Clause | https://code.videolan.org/videolan/dav1d |
| x264 | GPL-2.0-or-later | https://code.videolan.org/videolan/x264 |
| x265 | GPL-2.0-or-later | https://bitbucket.org/multicoreware/x265_git |
| SVT-AV1 | BSD-3-Clause-Clear, plus the AOMedia patent licence | https://gitlab.com/AOMediaCodec/SVT-AV1 |
| MKVToolNix | GPL-2.0 | https://codeberg.org/mbunkus/mkvtoolnix |
| libebml, libmatroska | LGPL-2.1-or-later | https://github.com/Matroska-Org |
| {fmt} | MIT | https://github.com/fmtlib/fmt |
| Boost | BSL-1.0 | https://github.com/boostorg/boost |
| MediaInfo, MediaInfoLib | BSD-2-Clause | https://github.com/MediaArea/MediaInfo |
| ZenLib | Zlib | https://github.com/MediaArea/ZenLib |
| Tesseract OCR (and the `eng` trained data) | Apache-2.0 | https://github.com/tesseract-ocr/tesseract |
| Leptonica | BSD-2-Clause | https://github.com/DanBloomberg/leptonica |
| dovi_tool | MIT | https://github.com/quietvoid/dovi_tool |
| hdr10plus_tool | MIT | https://github.com/quietvoid/hdr10plus_tool |
| VapourSynth | LGPL-2.1-or-later | https://github.com/vapoursynth/vapoursynth |

## VapourSynth plugins

The Flatpak includes these, and `packaging/scripts/install-vs-plugins.py` (run by the CachyOS installer) downloads the same official builds from PyPI into `~/.local/share/anvil/vs`. Licences are as declared on PyPI.

| Plugin | Licence | Source |
| --- | --- | --- |
| BestSource (`bs`) | MIT | https://github.com/vapoursynth/bestsource |
| L-SMASH Works (`lsmas`) | ISC and LGPL-2.1-or-later | https://github.com/HomeOfAviSynthPlusEvolution/L-SMASH-Works |
| d2vsource (`d2v`) | LGPL-2.1-or-later | https://github.com/dwbuiten/d2vsource |
| VIVTC (`vivtc`) | MIT | https://github.com/vapoursynth/vivtc |
| Bwdif (`bwdif`) | LGPL-3.0-or-later | https://github.com/HomeOfVapourSynthEvolution/VapourSynth-Bwdif |
| vszip (`vszip`) | MIT | https://pypi.org/project/vapoursynth-vszip/ |
| CAS (`cas`) | MIT | https://github.com/HolyWu/VapourSynth-CAS |
| AddNoise (`noise`, package `vsnoise`) | GPL-3.0 | https://pypi.org/project/vsnoise/ |
| znedi3 (`znedi3`) | GPL-2.0 | https://github.com/sekrit-twc/znedi3 |
| MVTools (`mv`) | GPL-2.0 | https://github.com/dubhatervapoursynth/vapoursynth-mvtools |

## Used if installed, never shipped

| Program | Licence | Source |
| --- | --- | --- |
| NVEncC, QSVEncC, VCEEncC | MIT (they use vendor SDKs under their own terms) | https://github.com/rigaya |
| rav1e | BSD-2-Clause | https://github.com/xiph/rav1e |
| havsfunc (QTGMC) | Unlicense (public domain) | https://github.com/HomeOfVapourSynthEvolution/havsfunc |

## Getting the source

Anvil's own source, including the scripts that build both packages, is in this repository. For the GPL and LGPL components shipped in the Flatpak, the manifest names the exact release or commit of each one, and `flatpak-builder` fetches exactly that source; for the AppImage, the Qt version is in the file names of `usr/lib/libQt6Core.so.*` inside it, and that release's source is at the Qt link above. If a link above stops working, open an issue and the source for any shipped version will be provided, as the GPL requires, for at least three years after that release.
