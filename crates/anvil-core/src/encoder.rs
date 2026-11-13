// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Video encoder command-line builders.
//!
//! Every encoder reads 10-bit y4m on stdin from `vspipe`, so filtering is
//! identical whichever encoder is chosen. Colour and HDR signalling is taken
//! from the probed source, never typed by the user.
//!
//! Full option pages (requirement 11) are driven by per-encoder schema files;
//! the settings structs here hold the handful of options the Quality Default
//! touches, plus `extra_args` for everything else.

use serde::{Deserialize, Serialize};

use crate::command::Command;
use crate::model::{DynamicRange, ResolutionClass, VideoTrack};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EncoderKind {
    #[serde(rename = "x265")]
    X265,
    #[serde(rename = "x264")]
    X264,
    #[serde(rename = "svt_av1")]
    SvtAv1,
    #[serde(rename = "rav1e")]
    Rav1e,
    #[serde(rename = "nvencc")]
    NvEncC,
    #[serde(rename = "qsvencc")]
    QsvEncC,
    #[serde(rename = "vceencc")]
    VceEncC,
    #[serde(rename = "ffmpeg_vaapi")]
    FfmpegVaapi,
}

impl EncoderKind {
    pub fn is_hardware(self) -> bool {
        matches!(
            self,
            EncoderKind::NvEncC
                | EncoderKind::QsvEncC
                | EncoderKind::VceEncC
                | EncoderKind::FfmpegVaapi
        )
    }

    /// Dolby Vision needs an HEVC encoder that writes the RPU itself with
    /// the right signalling (x265 and rigaya's NVEncC, QSVEncC, VCEEncC).
    /// FFmpeg's VAAPI encoders cannot.
    pub fn supports_dolby_vision(self, codec: &str) -> bool {
        codec == "hevc"
            && matches!(
                self,
                EncoderKind::X265
                    | EncoderKind::NvEncC
                    | EncoderKind::QsvEncC
                    | EncoderKind::VceEncC
            )
    }

    /// HDR10+ is injected into the encoded HEVC stream afterwards with
    /// hdr10plus_tool, so any HEVC encoder works, whether or not it was
    /// built with HDR10+ support (many x265 builds are not).
    pub fn supports_hdr10_plus(self, codec: &str) -> bool {
        codec == "hevc"
    }

    /// Whether the encoder writes HDR10 static metadata (mastering display
    /// and MaxCLL). FFmpeg's VAAPI encoders only copy it from decoded
    /// frames, and y4m carries none.
    pub fn writes_static_hdr(self) -> bool {
        !matches!(self, EncoderKind::FfmpegVaapi | EncoderKind::X264)
    }

    /// Output bit depth.
    pub fn output_depth(self, codec: &str) -> u8 {
        if self == EncoderKind::X264 || codec == "h264" {
            8
        } else {
            10
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EncoderKind::X265 => "x265",
            EncoderKind::X264 => "x264",
            EncoderKind::SvtAv1 => "SVT-AV1",
            EncoderKind::Rav1e => "rav1e",
            EncoderKind::NvEncC => "NVEncC",
            EncoderKind::QsvEncC => "QSVEncC",
            EncoderKind::VceEncC => "VCEEncC",
            EncoderKind::FfmpegVaapi => "FFmpeg VAAPI",
        }
    }

    pub const ALL: [EncoderKind; 8] = [
        EncoderKind::X265,
        EncoderKind::X264,
        EncoderKind::SvtAv1,
        EncoderKind::Rav1e,
        EncoderKind::NvEncC,
        EncoderKind::QsvEncC,
        EncoderKind::VceEncC,
        EncoderKind::FfmpegVaapi,
    ];
}

/// Elementary stream extension for a codec.
pub fn output_extension(codec: &str) -> &'static str {
    match codec {
        "h264" => "h264",
        "av1" => "ivf",
        _ => "hevc",
    }
}

/// "h265", "HEVC" -> "hevc"; "avc", "h264" -> "h264"; "av1".
pub fn normalise_codec(c: &str) -> String {
    match c.to_ascii_lowercase().as_str() {
        "h265" | "hevc" | "x265" => "hevc".into(),
        "h264" | "avc" | "x264" => "h264".into(),
        "av1" => "av1".into(),
        other => other.into(),
    }
}

/// A rate-control value chosen by source resolution class.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ByClass<T> {
    pub sd: T,
    pub hd: T,
    pub uhd: T,
}

impl<T: Copy> ByClass<T> {
    pub fn pick(&self, class: ResolutionClass) -> T {
        match class {
            ResolutionClass::Sd => self.sd,
            ResolutionClass::Hd => self.hd,
            ResolutionClass::Uhd => self.uhd,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct X265Settings {
    pub preset: String,
    pub crf: ByClass<f32>,
    #[serde(default)]
    pub tune: Option<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SvtAv1Settings {
    pub preset: u8,
    pub crf: ByClass<u8>,
    #[serde(default)]
    pub film_grain: Option<u8>,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NvEncSettings {
    pub codec: String,
    /// Constant-quality target (`--qvbr`).
    pub quality: ByClass<u8>,
    pub preset: String,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

/// Intel QSVEncC and AMD VCEEncC: codec, constant quality per class and
/// the quality/speed preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HwSettings {
    pub codec: String,
    /// QSVEncC `--icq`, VCEEncC `--cqp` (lower is better quality).
    pub quality: ByClass<u8>,
    pub preset: String,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

impl HwSettings {
    pub fn qsv_default() -> Self {
        HwSettings {
            codec: "hevc".into(),
            quality: ByClass {
                sd: 20,
                hd: 21,
                uhd: 22,
            },
            preset: "best".into(),
            extra_args: Vec::new(),
        }
    }
    pub fn vce_default() -> Self {
        HwSettings {
            codec: "hevc".into(),
            quality: ByClass {
                sd: 20,
                hd: 21,
                uhd: 22,
            },
            preset: "slow".into(),
            extra_args: Vec::new(),
        }
    }
}

/// FFmpeg VAAPI (Intel and AMD through Mesa or the Intel media driver).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VaapiSettings {
    pub codec: String,
    /// Constant QP per class.
    pub qp: ByClass<u8>,
    /// DRM render node.
    pub device: String,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

impl Default for VaapiSettings {
    fn default() -> Self {
        VaapiSettings {
            codec: "hevc".into(),
            qp: ByClass {
                sd: 21,
                hd: 22,
                uhd: 23,
            },
            device: "/dev/dri/renderD128".into(),
            extra_args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct X264Settings {
    pub preset: String,
    pub crf: ByClass<f32>,
    #[serde(default)]
    pub tune: Option<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

impl Default for X264Settings {
    fn default() -> Self {
        X264Settings {
            preset: "slow".into(),
            crf: ByClass {
                sd: 17.0,
                hd: 18.0,
                uhd: 19.0,
            },
            tune: Some("film".into()),
            extra_args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rav1eSettings {
    /// 0 (slowest) to 10 (fastest).
    pub speed: u8,
    /// 0 to 255, lower is better.
    pub quantizer: ByClass<u8>,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

impl Default for Rav1eSettings {
    fn default() -> Self {
        Rav1eSettings {
            speed: 6,
            quantizer: ByClass {
                sd: 70,
                hd: 80,
                uhd: 90,
            },
            extra_args: Vec::new(),
        }
    }
}

/// Per-job inputs that come from probing and earlier pipeline steps.
#[derive(Debug, Clone)]
pub struct EncodeInputs<'a> {
    pub video: &'a VideoTrack,
    /// Output resolution class after cropping (crop can move UHD scope to
    /// a smaller height, but the class follows the width).
    pub class: ResolutionClass,
    pub output: String,
    /// Dolby Vision RPU (already converted and crop-adjusted).
    pub dolby_vision_rpu: Option<String>,
}

fn known(v: &Option<String>) -> Option<&str> {
    v.as_deref()
        .filter(|s| !s.is_empty() && *s != "unknown" && *s != "reserved")
}

/// x265 `--chromaloc` value from ffprobe's chroma_location name.
fn chromaloc_index(name: &str) -> Option<u8> {
    match name {
        "left" => Some(0),
        "center" => Some(1),
        "topleft" => Some(2),
        "top" => Some(3),
        "bottomleft" => Some(4),
        "bottom" => Some(5),
        _ => None,
    }
}

pub fn x265_command(s: &X265Settings, input: &EncodeInputs) -> Command {
    let v = input.video;
    let mut c = Command::new("x265")
        .args(["--input", "-", "--y4m"])
        .args(["--preset", &s.preset])
        .args(["--crf", &format_crf(s.crf.pick(input.class))])
        .args(["--output-depth", "10", "--profile", "main10"]);
    if let Some(tune) = &s.tune {
        c = c.args(["--tune", tune]);
    }

    if let Some(p) = known(&v.colour.primaries) {
        c = c.args(["--colorprim", p]);
    }
    if let Some(t) = known(&v.colour.transfer) {
        c = c.args(["--transfer", t]);
    }
    if let Some(m) = known(&v.colour.matrix) {
        c = c.args(["--colormatrix", m]);
    }
    if let Some(r) = known(&v.colour.range) {
        c = c.args(["--range", if r == "pc" { "full" } else { "limited" }]);
    }
    if let Some(loc) = known(&v.colour.chroma_location).and_then(chromaloc_index) {
        c = c.args(["--chromaloc", &loc.to_string()]);
    }

    if v.dynamic_range == DynamicRange::Hdr10 {
        c = c.args(["--hdr10", "--hdr10-opt", "--repeat-headers"]);
        if let Some(meta) = &v.hdr10 {
            if let Some(md) = &meta.mastering_display {
                c = c.args(["--master-display", &md.to_x265()]);
            }
            if let Some(cll) = &meta.content_light {
                c = c.args(["--max-cll", &cll.to_arg()]);
            }
        }
    }

    if let Some(rpu) = &input.dolby_vision_rpu {
        // x265 needs VBV (HRD) for Dolby Vision. Level 5.1 High tier limits.
        c = c
            .args(["--dolby-vision-profile", "8.1", "--dolby-vision-rpu", rpu])
            .args(["--vbv-bufsize", "160000", "--vbv-maxrate", "160000"]);
    }

    c.args(s.extra_args.iter().cloned())
        .args(["--output", &input.output])
}

/// AV1 colour signalling uses the numeric H.273 code points.
fn h273_primaries(name: &str) -> Option<u8> {
    Some(match name {
        "bt709" => 1,
        "bt470bg" => 5,
        "smpte170m" => 6,
        "bt2020" => 9,
        _ => return None,
    })
}
fn h273_transfer(name: &str) -> Option<u8> {
    Some(match name {
        "bt709" => 1,
        "smpte170m" => 6,
        "smpte2084" => 16,
        "arib-std-b67" => 18,
        _ => return None,
    })
}
fn h273_matrix(name: &str) -> Option<u8> {
    Some(match name {
        "bt709" => 1,
        "bt470bg" => 5,
        "smpte170m" => 6,
        "bt2020nc" => 9,
        _ => return None,
    })
}

pub fn svt_av1_command(s: &SvtAv1Settings, input: &EncodeInputs) -> Command {
    let v = input.video;
    let mut c = Command::new("SvtAv1EncApp")
        .args(["-i", "stdin"])
        .args(["--preset", &s.preset.to_string()])
        .args(["--crf", &s.crf.pick(input.class).to_string()])
        .args(["--input-depth", "10"]);
    if let Some(g) = s.film_grain {
        c = c.args(["--film-grain", &g.to_string()]);
    }
    if let Some(p) = known(&v.colour.primaries).and_then(h273_primaries) {
        c = c.args(["--color-primaries", &p.to_string()]);
    }
    if let Some(t) = known(&v.colour.transfer).and_then(h273_transfer) {
        c = c.args(["--transfer-characteristics", &t.to_string()]);
    }
    if let Some(m) = known(&v.colour.matrix).and_then(h273_matrix) {
        c = c.args(["--matrix-coefficients", &m.to_string()]);
    }
    if v.dynamic_range == DynamicRange::Hdr10 {
        if let Some(meta) = &v.hdr10 {
            if let Some(md) = &meta.mastering_display {
                c = c.args(["--mastering-display", &md.to_svt_av1()]);
            }
            if let Some(cll) = &meta.content_light {
                c = c.args(["--content-light", &cll.to_arg()]);
            }
        }
    }
    c.args(s.extra_args.iter().cloned())
        .args(["-b", &input.output])
}

pub fn nvencc_command(s: &NvEncSettings, input: &EncodeInputs) -> Command {
    let codec = normalise_codec(&s.codec);
    let mut c = Command::new(crate::tools::program("NVEncC"))
        .args(["--y4m", "-i", "-"])
        .args(["--codec", &codec])
        .args(["--preset", &s.preset])
        .args(["--qvbr", &s.quality.pick(input.class).to_string()]);
    if codec != "h264" {
        c = c.args(["--output-depth", "10"]);
    }
    rigaya_common(c, input, &codec)
        .args(s.extra_args.iter().cloned())
        .args(["-o", &input.output])
}

/// Colour, HDR10 and Dolby Vision options shared by rigaya's encoders
/// (NVEncC, QSVEncC, VCEEncC use the same names).
fn rigaya_common(mut c: Command, input: &EncodeInputs, codec: &str) -> Command {
    let v = input.video;
    if let Some(p) = known(&v.colour.primaries) {
        c = c.args(["--colorprim", p]);
    }
    if let Some(t) = known(&v.colour.transfer) {
        c = c.args(["--transfer", t]);
    }
    if let Some(m) = known(&v.colour.matrix) {
        c = c.args(["--colormatrix", m]);
    }
    if let Some(r) = known(&v.colour.range) {
        c = c.args(["--colorrange", if r == "pc" { "full" } else { "limited" }]);
    }
    if let Some(loc) = known(&v.colour.chroma_location).and_then(chromaloc_index) {
        c = c.args(["--chromaloc", &loc.to_string()]);
    }
    if v.dynamic_range == DynamicRange::Hdr10 {
        if let Some(meta) = &v.hdr10 {
            if let Some(md) = &meta.mastering_display {
                c = c.args(["--master-display", &md.to_x265()]);
            }
            if let Some(cll) = &meta.content_light {
                c = c.args(["--max-cll", &cll.to_arg()]);
            }
        }
    }
    if codec == "hevc" {
        if let Some(rpu) = &input.dolby_vision_rpu {
            c = c.args(["--dolby-vision-profile", "8.1", "--dolby-vision-rpu", rpu]);
        }
    }
    c
}

/// Intel Quick Sync through rigaya's QSVEncC: intelligent constant quality.
pub fn qsvencc_command(s: &HwSettings, input: &EncodeInputs) -> Command {
    let codec = normalise_codec(&s.codec);
    let mut c = Command::new(crate::tools::program("QSVEncC"))
        .args(["--y4m", "-i", "-"])
        .args(["--codec", &codec])
        .args(["--quality", &s.preset])
        .args(["--icq", &s.quality.pick(input.class).to_string()]);
    if codec != "h264" {
        c = c.args(["--output-depth", "10"]);
        if codec == "hevc" {
            c = c.args(["--profile", "main10"]);
        }
    }
    rigaya_common(c, input, &codec)
        .args(s.extra_args.iter().cloned())
        .args(["-o", &input.output])
}

/// AMD AMF through rigaya's VCEEncC: constant QP.
pub fn vceencc_command(s: &HwSettings, input: &EncodeInputs) -> Command {
    let codec = normalise_codec(&s.codec);
    let q = s.quality.pick(input.class);
    let mut c = Command::new(crate::tools::program("VCEEncC"))
        .args(["--y4m", "-i", "-"])
        .args(["--codec", &codec])
        .args(["--preset", &s.preset])
        // I, P, B: B-frames a little coarser, as x265's ratios.
        .args(["--cqp", &format!("{q}:{}:{}", q + 2, q + 4)]);
    if codec != "h264" {
        c = c.args(["--output-depth", "10"]);
    }
    rigaya_common(c, input, &codec)
        .args(s.extra_args.iter().cloned())
        .args(["-o", &input.output])
}

/// FFmpeg's VAAPI encoders: frames are uploaded to the GPU and encoded at
/// a constant QP. Colours are signalled; HDR10 static metadata is not.
pub fn vaapi_command(s: &VaapiSettings, input: &EncodeInputs) -> Command {
    let v = input.video;
    let codec = normalise_codec(&s.codec);
    let (encoder, upload, muxer) = match codec.as_str() {
        "h264" => ("h264_vaapi", "format=nv12,hwupload", "h264"),
        "av1" => ("av1_vaapi", "format=p010le,hwupload", "ivf"),
        _ => ("hevc_vaapi", "format=p010le,hwupload", "hevc"),
    };
    let mut c = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-y"])
        .args(["-vaapi_device", &s.device])
        .args(["-f", "yuv4mpegpipe", "-i", "-"])
        .args(["-vf", upload])
        .args(["-c:v", encoder])
        .args([
            "-rc_mode",
            "CQP",
            "-qp",
            &s.qp.pick(input.class).to_string(),
        ]);
    if codec == "hevc" {
        c = c.args(["-profile:v", "main10"]);
    }
    if let Some(p) = known(&v.colour.primaries) {
        c = c.args(["-color_primaries", p]);
    }
    if let Some(t) = known(&v.colour.transfer) {
        c = c.args(["-color_trc", t]);
    }
    if let Some(m) = known(&v.colour.matrix) {
        // FFmpeg's names for the matrix: bt2020nc, bt709, smpte170m...
        c = c.args(["-colorspace", m]);
    }
    if let Some(r) = known(&v.colour.range) {
        c = c.args(["-color_range", if r == "pc" { "pc" } else { "tv" }]);
    }
    c.args(s.extra_args.iter().cloned())
        .args(["-f", muxer, &input.output])
}

/// x264: 8-bit AVC for players that cannot do HEVC. SDR only.
pub fn x264_command(s: &X264Settings, input: &EncodeInputs) -> Command {
    let v = input.video;
    let mut c = Command::new("x264")
        .args(["--demuxer", "y4m", "-"])
        .args(["--preset", &s.preset])
        .args(["--crf", &format_crf(s.crf.pick(input.class))])
        .args(["--output-depth", "8"]);
    if let Some(tune) = &s.tune {
        c = c.args(["--tune", tune]);
    }
    if let Some(p) = known(&v.colour.primaries) {
        c = c.args(["--colorprim", p]);
    }
    if let Some(t) = known(&v.colour.transfer) {
        c = c.args(["--transfer", t]);
    }
    if let Some(m) = known(&v.colour.matrix) {
        c = c.args(["--colormatrix", m]);
    }
    if let Some(r) = known(&v.colour.range) {
        c = c.args(["--range", if r == "pc" { "pc" } else { "tv" }]);
    }
    if let Some(loc) = known(&v.colour.chroma_location).and_then(chromaloc_index) {
        c = c.args(["--chromaloc", &loc.to_string()]);
    }
    c.args(s.extra_args.iter().cloned())
        .args(["--output", &input.output])
}

fn rav1e_name(kind: &str, name: &str) -> Option<&'static str> {
    Some(match (kind, name) {
        ("primaries", "bt709") => "BT709",
        ("primaries", "bt2020") => "BT2020",
        ("primaries", "bt470bg") => "BT470BG",
        ("primaries", "smpte170m") => "BT601",
        ("transfer", "bt709") => "BT709",
        ("transfer", "smpte2084") => "SMPTE2084",
        ("transfer", "arib-std-b67") => "HLG",
        ("transfer", "smpte170m") => "BT601",
        ("matrix", "bt709") => "BT709",
        ("matrix", "bt2020nc") => "BT2020NCL",
        ("matrix", "bt470bg") => "BT470BG",
        ("matrix", "smpte170m") => "BT601",
        _ => return None,
    })
}

/// rav1e: AV1 from y4m on stdin, 10-bit, HDR10 signalled.
pub fn rav1e_command(s: &Rav1eSettings, input: &EncodeInputs) -> Command {
    let v = input.video;
    let mut c = Command::new("rav1e")
        .args(["-", "--speed", &s.speed.to_string()])
        .args(["--quantizer", &s.quantizer.pick(input.class).to_string()]);
    if let Some(p) = known(&v.colour.primaries).and_then(|n| rav1e_name("primaries", n)) {
        c = c.args(["--primaries", p]);
    }
    if let Some(t) = known(&v.colour.transfer).and_then(|n| rav1e_name("transfer", n)) {
        c = c.args(["--transfer", t]);
    }
    if let Some(m) = known(&v.colour.matrix).and_then(|n| rav1e_name("matrix", n)) {
        c = c.args(["--matrix", m]);
    }
    if let Some(r) = known(&v.colour.range) {
        c = c.args(["--range", if r == "pc" { "Full" } else { "Limited" }]);
    }
    if v.dynamic_range == DynamicRange::Hdr10 {
        if let Some(meta) = &v.hdr10 {
            if let Some(md) = &meta.mastering_display {
                c = c.args(["--mastering-display", &md.to_svt_av1()]);
            }
            if let Some(cll) = &meta.content_light {
                c = c.args(["--content-light", &cll.to_arg()]);
            }
        }
    }
    c.args(s.extra_args.iter().cloned())
        .args(["--output", &input.output])
}

fn format_crf(v: f32) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v}")
    }
}

/// Reject combinations that would silently degrade the output.
pub fn preflight(kind: EncoderKind, codec: &str, video: &VideoTrack) -> Result<()> {
    if video.is_hdr() && (kind == EncoderKind::X264 || codec == "h264") {
        return Err(Error::Unsupported {
            encoder: kind.label().into(),
            reason: "HDR sources need a 10-bit HEVC or AV1 encoder; 8-bit AVC would drop HDR"
                .into(),
        });
    }
    if !["hevc", "av1", "h264"].contains(&codec) {
        return Err(Error::Unsupported {
            encoder: kind.label().into(),
            reason: format!("unknown codec '{codec}' (use hevc, av1 or h264)"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uhd() -> crate::model::SourceInfo {
        crate::probe::parse_ffprobe_json(include_str!("../tests/fixtures/uhd_dovi_p7_truehd.json"))
            .unwrap()
    }

    fn inputs(v: &VideoTrack, rpu: bool) -> EncodeInputs<'_> {
        EncodeInputs {
            video: v,
            class: ResolutionClass::Uhd,
            output: "out.hevc".into(),
            dolby_vision_rpu: rpu.then(|| "RPU.bin".to_string()),
        }
    }

    #[test]
    fn rigaya_encoders_share_hdr_and_dolby_vision() {
        let s = uhd();
        let qsv = qsvencc_command(&HwSettings::qsv_default(), &inputs(&s.video, true)).to_shell();
        for want in [
            "QSVEncC --y4m -i -",
            "--codec hevc",
            "--quality best",
            "--icq 22",
            "--output-depth 10",
            "--profile main10",
            "--colorprim bt2020",
            "--transfer smpte2084",
            "--master-display",
            "--max-cll 1105,307",
            "--dolby-vision-profile 8.1 --dolby-vision-rpu RPU.bin",
            "-o out.hevc",
        ] {
            assert!(qsv.contains(want), "{want} missing from {qsv}");
        }
        let vce = vceencc_command(&HwSettings::vce_default(), &inputs(&s.video, true)).to_shell();
        assert!(
            vce.contains("--cqp 22:24:26")
                && vce.contains("--preset slow")
                && vce.contains("--dolby-vision-rpu"),
            "{vce}"
        );
        // AV1 on a GPU: no Dolby Vision RPU option.
        let mut av1 = HwSettings::qsv_default();
        av1.codec = "AV1".into();
        let q = qsvencc_command(&av1, &inputs(&s.video, true)).to_shell();
        assert!(
            q.contains("--codec av1") && !q.contains("dolby") && !q.contains("main10"),
            "{q}"
        );
        assert!(EncoderKind::QsvEncC.supports_dolby_vision("hevc"));
        assert!(!EncoderKind::QsvEncC.supports_dolby_vision("av1"));
        assert!(!EncoderKind::FfmpegVaapi.supports_dolby_vision("hevc"));
        assert!(EncoderKind::FfmpegVaapi.supports_hdr10_plus("hevc"));
    }

    #[test]
    fn vaapi_x264_rav1e() {
        let s = uhd();
        let va = vaapi_command(&VaapiSettings::default(), &inputs(&s.video, false)).to_shell();
        for want in [
            "-vaapi_device /dev/dri/renderD128",
            "-f yuv4mpegpipe -i -",
            "format=p010le,hwupload",
            "-c:v hevc_vaapi",
            "-rc_mode CQP -qp 23",
            "-profile:v main10",
            "-color_primaries bt2020",
            "-color_trc smpte2084",
            "-colorspace bt2020nc",
            "-f hevc out.hevc",
        ] {
            assert!(va.contains(want), "{want} missing from {va}");
        }
        let r = rav1e_command(&Rav1eSettings::default(), &inputs(&s.video, false)).to_shell();
        for want in [
            "rav1e - --speed 6",
            "--quantizer 90",
            "--primaries BT2020",
            "--transfer SMPTE2084",
            "--matrix BT2020NCL",
            "--mastering-display",
            "--content-light 1105,307",
        ] {
            assert!(r.contains(want), "{want} missing from {r}");
        }
        assert!(
            preflight(EncoderKind::X264, "h264", &s.video).is_err(),
            "HDR to 8-bit AVC refused"
        );
        assert!(preflight(EncoderKind::NvEncC, "h264", &s.video).is_err());
        assert!(preflight(EncoderKind::NvEncC, "vp9", &s.video).is_err());
        assert_eq!(output_extension("av1"), "ivf");
        assert_eq!(normalise_codec("H265"), "hevc");
    }
}
