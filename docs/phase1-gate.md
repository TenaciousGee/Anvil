# Phase 1 gate: results

**Gate:** a UHD HDR10 file re-encodes from the CLI with correct HDR metadata, checked by ffprobe and MediaInfo.

**Result: passed** (10 November 2026), on three real files encoded with `anvil run --autocrop`.

## Test sources

Authored to match what a MakeMKV rip of a UHD Blu-ray contains, rather than relying on fixtures:

| Source | Video | Audio | Other |
| --- | --- | --- | --- |
| `UHD Film (2026)/title_t00.mkv` | 3840x2160 HEVC 10-bit, 280 px letterbox, HDR10 (1000 nits, MaxCLL 1000/400), Dolby Vision 8.1 RPU with matching L5 offsets, HDR10+ (259 frames of metadata) | TrueHD 5.1, AC-3 5.1 640k | SRT subtitles, 3 chapters |
| `BDMV Film (2026)/00800.m2ts` | Same video in an M2TS (Dolby Vision signalled only in the frames) | TrueHD 5.1 (with embedded AC-3 core), AC-3 5.1 | |
| `My Film (2026)/title_t00.mkv` | 1280x720 H.264 8-bit SDR, 88 px letterbox | FLAC, AC-3 | |

The Dolby Vision RPU was produced with `dovi_tool generate`, HDR10+ was injected with `hdr10plus_tool inject` using its sample metadata, and the source was encoded with x265 (`--hdr10`, `--master-display`, `--max-cll`, `--dolby-vision-profile 8.1`).

To keep 4K encodes to minutes on a 2-core test machine, the runs used a user preset that inherits Quality Default and changes only `video.x265.preset` to `ultrafast`. Every HDR, crop, audio and mux step is identical to Quality Default.

## Dolby Vision crop check

`dovi_tool info -s` on the RPU before and after:

- Source: `L5 offsets: top=280, bottom=280, left=0, right=0`
- Output (cropped to 3840x1600): `L5 offsets: top=0, bottom=0, left=0, right=0`, 259 frames, profile 8

## Verification output

```
=================== UHD Film (2026)/title_t00.mkv
Keeps:   HDR10 · Dolby Vision 8.1 · HDR10+ · TrueHD · AC-3
  ✔ Video codec                            expected hevc, found hevc
  ✔ Picture size                           expected 3840x1600, found 3840x1600
  ✔ Bit depth                              expected 10-bit, found 10-bit
  ✔ Duration                               expected 10.82s, found 10.82s
  ✔ Dynamic range                          expected Hdr10, found Hdr10
  ✔ HDR10 mastering display                present, max 1000 nits
  ✔ HDR10 MaxCLL / MaxFALL                 1000 / 400
  ✔ Dolby Vision                           profile 8
  ✔ HDR10+                                 present
  ✔ Audio tracks and order                 expected Dolby TrueHD, Dolby Digital (AC-3), found Dolby TrueHD, Dolby Digital (AC-3)
  ✔ Default audio track                    first track is default
  ✔ Subtitles                              expected 1, found 1
  ✔ Chapters                               3 chapters
  ✔ MediaInfo: bit depth                   10 bits
  ✔ MediaInfo: HDR10+ (SMPTE ST 2094-40)   Dolby Vision / SMPTE ST 2094 App 4 dvhe.08 /  HDR10 / HDR10+ Profile A
Done in 0:36: …/final/title_t00.anvil.mkv
exit=0
=================== BDMV Film (2026)/00800.m2ts
Keeps:   HDR10 · Dolby Vision 8.1 · HDR10+ · TrueHD · AC-3
  ✔ Video codec                            expected hevc, found hevc
  ✔ Picture size                           expected 3840x1600, found 3840x1600
  ✔ Bit depth                              expected 10-bit, found 10-bit
  ✔ Duration                               expected 10.82s, found 10.82s
  ✔ Dynamic range                          expected Hdr10, found Hdr10
  ✔ HDR10 mastering display                present, max 1000 nits
  ✔ HDR10 MaxCLL / MaxFALL                 1000 / 400
  ✔ Dolby Vision                           profile 8
  ✔ HDR10+                                 present
  ✔ Audio tracks and order                 expected Dolby TrueHD, Dolby Digital (AC-3), found Dolby TrueHD, Dolby Digital (AC-3)
  ✔ Default audio track                    first track is default
  ✔ Subtitles                              expected 0, found 0
  ✔ MediaInfo: bit depth                   10 bits
  ✔ MediaInfo: HDR10+ (SMPTE ST 2094-40)   Dolby Vision / SMPTE ST 2094 App 4 dvhe.08 /  HDR10 / HDR10+ Profile A
Done in 0:37: …/final/00800.anvil.mkv
exit=0
=================== My Film (2026)/title_t00.mkv
Keeps:   FLAC · AC-3
  ✔ Video codec                            expected hevc, found hevc
  ✔ Picture size                           expected 1280x544, found 1280x544
  ✔ Bit depth                              expected 10-bit, found 10-bit
  ✔ Duration                               expected 5.00s, found 5.02s
  ✔ Dynamic range                          expected Sdr, found Sdr
  ✔ Audio tracks and order                 expected FLAC, Dolby Digital (AC-3), found FLAC, Dolby Digital (AC-3)
  ✔ Default audio track                    first track is default
  ✔ Subtitles                              expected 0, found 0
  ✔ MediaInfo: bit depth                   10 bits
Done in 0:02: …/final/title_t00.anvil.mkv
exit=0
```
