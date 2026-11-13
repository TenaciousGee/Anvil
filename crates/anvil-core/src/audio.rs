// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! The audio rule.
//!
//! Default order for the primary language is always:
//!   1. the native lossless track (Dolby TrueHD, DTS-HD MA or LPCM), passed through;
//!   2. Dolby Digital (AC-3): the disc's own AC-3 track passed through when it
//!      has one, otherwise encoded from the lossless track.
//!
//! Users can reorder per file with [`move_track`]; the reorder lives in the
//! job, never in the preset, so the default stays lossless then AC-3.

use serde::{Deserialize, Serialize};

use crate::model::{channel_layout, AudioCodec, AudioTrack};
use crate::{Error, Result};

/// Audio settings carried by a preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioRules {
    /// Preferred languages in order (ISO 639-2, e.g. "eng"). The first one
    /// present gets the lossless + AC-3 pair; later ones get their best track.
    /// Empty means "the source's default track language".
    pub languages: Vec<String>,
    /// AC-3 bitrate for 5.1 encodes (kbps). 640 is the format maximum.
    pub ac3_bitrate_multichannel: u32,
    /// AC-3 bitrate for mono and stereo encodes (kbps).
    pub ac3_bitrate_stereo: u32,
}

impl Default for AudioRules {
    fn default() -> Self {
        AudioRules {
            languages: Vec::new(),
            ac3_bitrate_multichannel: 640,
            ac3_bitrate_stereo: 224,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioTarget {
    Ac3,
    Eac3,
    Aac,
    Opus,
    Flac,
    /// The lossy DTS core inside a DTS-HD track, copied out bit for bit.
    DtsCore,
}

impl AudioTarget {
    pub const ALL: [AudioTarget; 6] = [
        AudioTarget::Ac3,
        AudioTarget::Eac3,
        AudioTarget::Aac,
        AudioTarget::Opus,
        AudioTarget::Flac,
        AudioTarget::DtsCore,
    ];

    /// FFmpeg encoder name ("copy" for the DTS core, which is extracted).
    pub fn ffmpeg_codec(self) -> &'static str {
        match self {
            AudioTarget::Ac3 => "ac3",
            AudioTarget::Eac3 => "eac3",
            AudioTarget::Aac => "aac",
            AudioTarget::Opus => "libopus",
            AudioTarget::Flac => "flac",
            AudioTarget::DtsCore => "copy",
        }
    }

    pub fn file_extension(self) -> &'static str {
        match self {
            AudioTarget::Ac3 => "ac3",
            AudioTarget::Eac3 => "eac3",
            AudioTarget::Aac => "m4a",
            AudioTarget::Opus => "opus",
            AudioTarget::Flac => "flac",
            AudioTarget::DtsCore => "dts",
        }
    }

    /// Stable id for the GUI and job files.
    pub fn id(self) -> &'static str {
        match self {
            AudioTarget::Ac3 => "ac3",
            AudioTarget::Eac3 => "eac3",
            AudioTarget::Aac => "aac",
            AudioTarget::Opus => "opus",
            AudioTarget::Flac => "flac",
            AudioTarget::DtsCore => "dts_core",
        }
    }

    pub fn from_id(id: &str) -> Option<AudioTarget> {
        AudioTarget::ALL.into_iter().find(|t| t.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            AudioTarget::Ac3 => "Dolby Digital (AC-3)",
            AudioTarget::Eac3 => "Dolby Digital Plus (E-AC-3)",
            AudioTarget::Aac => "AAC",
            AudioTarget::Opus => "Opus",
            AudioTarget::Flac => "FLAC (lossless)",
            AudioTarget::DtsCore => "DTS core (no re-encode)",
        }
    }

    /// Short name: "AC-3", "Opus".
    pub fn short(self) -> &'static str {
        match self {
            AudioTarget::Ac3 => "AC-3",
            AudioTarget::Eac3 => "E-AC-3",
            AudioTarget::Aac => "AAC",
            AudioTarget::Opus => "Opus",
            AudioTarget::Flac => "FLAC",
            AudioTarget::DtsCore => "DTS core",
        }
    }

    /// The codec the output track will have.
    pub fn codec(self) -> AudioCodec {
        match self {
            AudioTarget::Ac3 => AudioCodec::Ac3,
            AudioTarget::Eac3 => AudioCodec::Eac3,
            AudioTarget::Aac => AudioCodec::Aac,
            AudioTarget::Opus => AudioCodec::Opus,
            AudioTarget::Flac => AudioCodec::Flac,
            AudioTarget::DtsCore => AudioCodec::Dts,
        }
    }

    /// FLAC and the DTS core have no bitrate to choose.
    pub fn has_bitrate(self) -> bool {
        !matches!(self, AudioTarget::Flac | AudioTarget::DtsCore)
    }

    /// Re-encodes the audio (so loudness and downmix apply).
    pub fn reencodes(self) -> bool {
        self != AudioTarget::DtsCore
    }

    /// Most channels the FFmpeg encoder accepts.
    pub fn max_channels(self) -> u32 {
        match self {
            // FFmpeg's AC-3 and E-AC-3 encoders stop at 5.1.
            AudioTarget::Ac3 | AudioTarget::Eac3 => 6,
            _ => 8,
        }
    }

    /// A sensible bitrate (kb/s) for this many channels.
    pub fn default_bitrate(self, channels: u32) -> u32 {
        let stereo = channels <= 2;
        match self {
            AudioTarget::Ac3 => {
                if stereo {
                    224
                } else {
                    640
                }
            }
            AudioTarget::Eac3 => {
                if stereo {
                    256
                } else {
                    1024
                }
            }
            AudioTarget::Aac => {
                if stereo {
                    192
                } else {
                    448
                }
            }
            AudioTarget::Opus => {
                if stereo {
                    160
                } else {
                    384
                }
            }
            AudioTarget::Flac | AudioTarget::DtsCore => 0,
        }
    }

    /// Bitrate limits (kb/s) for the encoder.
    pub fn bitrate_range(self) -> (u32, u32) {
        match self {
            AudioTarget::Ac3 => (96, 640),
            AudioTarget::Eac3 => (96, 1536),
            AudioTarget::Aac => (64, 1024),
            AudioTarget::Opus => (48, 768),
            AudioTarget::Flac | AudioTarget::DtsCore => (0, 0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AudioAction {
    Passthrough,
    Encode {
        target: AudioTarget,
        bitrate_kbps: u32,
        channels: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioRole {
    /// Native lossless (or best available) track for the primary language.
    Primary,
    /// Dolby Digital compatibility track for the primary language.
    Compatibility,
    /// Best track for an additional preferred language.
    Additional,
}

/// One output audio track, in output order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioPlan {
    /// Container stream index of the source track.
    pub source_index: u32,
    /// Position among the source's audio streams (`0:a:N` in FFmpeg).
    pub source_audio_ordinal: u32,
    pub source_codec: AudioCodec,
    pub action: AudioAction,
    pub role: AudioRole,
    pub language: Option<String>,
    pub title: Option<String>,
    pub default: bool,
    /// Shift the track by this many milliseconds (positive plays later).
    #[serde(default)]
    pub delay_ms: i32,
    /// Normalise to this integrated loudness (LUFS, EBU R128) with one fixed
    /// gain for the whole track, so the mix's dynamics are untouched.
    /// Re-encoded tracks only.
    #[serde(default)]
    pub loudness_lufs: Option<f64>,
}

/// Default target for loudness normalisation: EBU R128.
pub const DEFAULT_LOUDNESS_LUFS: f64 = -23.0;
/// Highest true peak allowed after the gain (dBTP).
pub const MAX_TRUE_PEAK_DB: f64 = -1.0;

impl AudioPlan {
    pub fn describe(&self) -> String {
        let mut s = match &self.action {
            AudioAction::Passthrough => format!("{} passthrough", self.source_codec.display_name()),
            AudioAction::Encode {
                target: AudioTarget::DtsCore,
                ..
            } => format!("DTS core from {}", self.source_codec.display_name()),
            AudioAction::Encode {
                target,
                bitrate_kbps,
                channels,
            } if !target.has_bitrate() => format!(
                "{} {} from {}",
                target.short(),
                channel_layout(*channels),
                self.source_codec.display_name()
            ),
            AudioAction::Encode {
                target,
                bitrate_kbps,
                channels,
            } => format!(
                "{} {} at {} kb/s from {}",
                target.short(),
                channel_layout(*channels),
                bitrate_kbps,
                self.source_codec.display_name()
            ),
        };
        if let Some(l) = self.loudness_lufs {
            s.push_str(&format!(", normalised to {l} LUFS"));
        }
        if self.delay_ms != 0 {
            s.push_str(&format!(", delayed {:+} ms", self.delay_ms));
        }
        s
    }

    /// The codec the output track will have.
    pub fn output_codec(&self) -> AudioCodec {
        match &self.action {
            AudioAction::Passthrough => self.source_codec,
            AudioAction::Encode { target, .. } => target.codec(),
        }
    }
}

/// What a user asked for one output track in the editor.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackEdit {
    /// `None` keeps the source track as it is.
    pub target: Option<AudioTarget>,
    pub bitrate_kbps: u32,
    pub channels: u32,
    pub delay_ms: i32,
    pub normalise: bool,
    pub title: Option<String>,
}

/// Apply an edit to one output track, checking it against the source track.
pub fn edit_track(
    plan: &mut [AudioPlan],
    index: usize,
    source: &AudioTrack,
    edit: &TrackEdit,
) -> Result<()> {
    let p = plan.get_mut(index).ok_or(Error::Invalid {
        field: "audio track",
        reason: format!("no output track {}", index + 1),
    })?;
    if edit.delay_ms.abs() > 10_000 {
        return Err(Error::Invalid {
            field: "audio delay",
            reason: "keep the delay within 10 seconds either way".into(),
        });
    }
    p.action = match edit.target {
        None => AudioAction::Passthrough,
        Some(AudioTarget::DtsCore) => {
            if !matches!(source.codec, AudioCodec::DtsHdMa | AudioCodec::DtsHdHra) {
                return Err(Error::Invalid {
                    field: "audio target",
                    reason: format!(
                        "only DTS-HD tracks carry a DTS core; this one is {}",
                        source.codec.display_name()
                    ),
                });
            }
            AudioAction::Encode {
                target: AudioTarget::DtsCore,
                bitrate_kbps: 0,
                channels: source.channels.min(6),
            }
        }
        Some(target) => {
            let channels = edit
                .channels
                .clamp(1, source.channels.max(1).min(target.max_channels()));
            let bitrate_kbps = if target.has_bitrate() {
                let (lo, hi) = target.bitrate_range();
                if edit.bitrate_kbps == 0 {
                    target.default_bitrate(channels)
                } else {
                    edit.bitrate_kbps.clamp(lo, hi)
                }
            } else {
                0
            };
            AudioAction::Encode {
                target,
                bitrate_kbps,
                channels,
            }
        }
    };
    let reencodes = matches!(&p.action, AudioAction::Encode { target, .. } if target.reencodes());
    p.loudness_lufs = (edit.normalise && reencodes).then_some(DEFAULT_LOUDNESS_LUFS);
    p.delay_ms = edit.delay_ms;
    if let Some(t) = &edit.title {
        p.title = (!t.trim().is_empty()).then(|| t.trim().to_string());
    }
    Ok(())
}

/// Build one output track from a command-line spec:
/// `N[:TARGET[:KBPS[:CHANNELS]]][,norm][,delay=MS][,title=TEXT]`, where N is
/// the source audio track (1 = first) and TARGET is copy, ac3, eac3, aac,
/// opus, flac or dts_core. Examples: `1`, `1:ac3:640`, `2:opus:160:2,norm`.
pub fn parse_track_spec(spec: &str, source: &[AudioTrack]) -> Result<AudioPlan> {
    let bad = |reason: String| Error::Invalid {
        field: "--audio",
        reason,
    };
    let mut parts = spec.split(',');
    let head = parts.next().unwrap_or_default();
    let mut fields = head.split(':');
    let n: usize = fields
        .next()
        .and_then(|f| f.trim().parse().ok())
        .filter(|n| *n >= 1)
        .ok_or_else(|| {
            bad(format!(
                "'{spec}': start with the source track number (1 = first)"
            ))
        })?;
    let track = source.get(n - 1).ok_or_else(|| {
        bad(format!(
            "'{spec}': the source has {} audio tracks",
            source.len()
        ))
    })?;
    let target = match fields.next().map(str::trim) {
        None | Some("") | Some("copy") => None,
        Some(id) => Some(AudioTarget::from_id(id).ok_or_else(|| {
            bad(format!(
                "'{id}' is not a target (copy, ac3, eac3, aac, opus, flac, dts_core)"
            ))
        })?),
    };
    let num = |f: Option<&str>| -> Result<u32> {
        f.map(str::trim)
            .filter(|f| !f.is_empty())
            .map_or(Ok(0), |f| {
                f.parse().map_err(|_| bad(format!("'{f}' is not a number")))
            })
    };
    let bitrate_kbps = num(fields.next())?;
    let channels = match num(fields.next())? {
        0 => track.channels,
        c => c,
    };
    let mut edit = TrackEdit {
        target,
        bitrate_kbps,
        channels,
        delay_ms: 0,
        normalise: false,
        title: None,
    };
    for opt in parts {
        let opt = opt.trim();
        if opt == "norm" {
            edit.normalise = true;
        } else if let Some(ms) = opt.strip_prefix("delay=") {
            edit.delay_ms = ms
                .parse()
                .map_err(|_| bad(format!("'{ms}' is not a delay in ms")))?;
        } else if let Some(t) = opt.strip_prefix("title=") {
            edit.title = Some(t.to_string());
        } else {
            return Err(bad(format!(
                "unknown option '{opt}' (norm, delay=MS, title=TEXT)"
            )));
        }
    }
    let mut plan = vec![passthrough(track, AudioRole::Additional)];
    edit_track(&mut plan, 0, track, &edit)?;
    let mut p = plan.remove(0);
    if p.title.is_none() || target.is_some() && edit.title.is_none() {
        // A re-encoded track gets a plain title rather than the source's.
        if let AudioAction::Encode {
            target, channels, ..
        } = &p.action
        {
            p.title = Some(format!(
                "{} {}",
                target.label().split(" (").next().unwrap_or(""),
                channel_layout(*channels)
            ));
        }
    }
    Ok(p)
}

/// Output tracks from specs, in order; the first is the default track.
pub fn plan_from_specs(specs: &[String], source: &[AudioTrack]) -> Result<Vec<AudioPlan>> {
    let mut plan = specs
        .iter()
        .map(|s| parse_track_spec(s, source))
        .collect::<Result<Vec<_>>>()?;
    set_default_flags(&mut plan);
    Ok(plan)
}

/// Add a source track to the end of the output, passed through.
pub fn add_track(plan: &mut Vec<AudioPlan>, source: &AudioTrack) {
    plan.push(passthrough(source, AudioRole::Additional));
    set_default_flags(plan);
}

/// Remove an output track.
pub fn remove_track(plan: &mut Vec<AudioPlan>, index: usize) -> Result<()> {
    if index >= plan.len() {
        return Err(Error::Invalid {
            field: "audio track",
            reason: format!("no output track {}", index + 1),
        });
    }
    plan.remove(index);
    set_default_flags(plan);
    Ok(())
}

/// Build the default audio plan for a source.
pub fn default_audio_plan(tracks: &[AudioTrack], rules: &AudioRules) -> Vec<AudioPlan> {
    // AC-3 cores embedded in TrueHD (M2TS) cannot be muxed on their own.
    let usable: Vec<AudioTrack> = tracks
        .iter()
        .filter(|t| !t.embedded_core)
        .cloned()
        .collect();
    let tracks = usable.as_slice();
    if tracks.is_empty() {
        return Vec::new();
    }

    let primary_lang = rules
        .languages
        .iter()
        .find(|l| {
            tracks
                .iter()
                .any(|t| t.language.as_deref() == Some(l.as_str()))
        })
        .cloned()
        .or_else(|| {
            tracks
                .iter()
                .find(|t| t.default)
                .and_then(|t| t.language.clone())
        })
        .or_else(|| tracks[0].language.clone());

    let primary_tracks: Vec<&AudioTrack> = tracks
        .iter()
        .filter(|t| t.language == primary_lang)
        .collect();
    // Untagged sources: everything counts as the primary language.
    let primary_tracks = if primary_tracks.is_empty() {
        tracks.iter().collect()
    } else {
        primary_tracks
    };

    let mut plan = primary_pair(&primary_tracks, rules);

    for lang in &rules.languages {
        if Some(lang) == primary_lang.as_ref() {
            continue;
        }
        let candidates: Vec<&AudioTrack> = tracks
            .iter()
            .filter(|t| t.language.as_deref() == Some(lang.as_str()))
            .collect();
        if let Some(best) = best_track(&candidates) {
            plan.push(passthrough(best, AudioRole::Additional));
        }
    }

    set_default_flags(&mut plan);
    plan
}

fn primary_pair(tracks: &[&AudioTrack], rules: &AudioRules) -> Vec<AudioPlan> {
    let mut plan = Vec::new();
    let Some(best) = best_track(tracks) else {
        return plan;
    };

    // If the best thing on offer is already AC-3, a single track satisfies
    // both slots.
    if best.codec == AudioCodec::Ac3 {
        plan.push(passthrough(best, AudioRole::Compatibility));
        return plan;
    }
    plan.push(passthrough(best, AudioRole::Primary));

    let existing_ac3 = tracks
        .iter()
        .filter(|t| t.codec == AudioCodec::Ac3)
        .max_by_key(|t| t.channels);

    match existing_ac3 {
        Some(ac3) => plan.push(passthrough(ac3, AudioRole::Compatibility)),
        None => {
            let channels = best.channels.min(6);
            let bitrate_kbps = if channels <= 2 {
                rules.ac3_bitrate_stereo
            } else {
                rules.ac3_bitrate_multichannel
            };
            plan.push(AudioPlan {
                source_index: best.index,
                source_audio_ordinal: best.audio_ordinal,
                source_codec: best.codec,
                action: AudioAction::Encode {
                    target: AudioTarget::Ac3,
                    bitrate_kbps,
                    channels,
                },
                role: AudioRole::Compatibility,
                language: best.language.clone(),
                title: Some(format!("Dolby Digital {}", channel_layout(channels))),
                default: false,
                delay_ms: 0,
                loudness_lufs: None,
            });
        }
    }
    plan
}

/// Lossless beats lossy; within each, format rank then channel count, then
/// the earliest stream wins (discs list the main track first).
fn best_track<'a>(tracks: &[&'a AudioTrack]) -> Option<&'a AudioTrack> {
    tracks.iter().copied().min_by_key(|t| {
        let (class, rank) = if t.codec.is_lossless() {
            (0u8, t.codec.lossless_rank())
        } else {
            (1u8, t.codec.lossy_rank())
        };
        (class, rank, std::cmp::Reverse(t.channels), t.index)
    })
}

fn passthrough(t: &AudioTrack, role: AudioRole) -> AudioPlan {
    AudioPlan {
        source_index: t.index,
        source_audio_ordinal: t.audio_ordinal,
        source_codec: t.codec,
        action: AudioAction::Passthrough,
        role,
        language: t.language.clone(),
        title: t.title.clone(),
        default: false,
        delay_ms: 0,
        loudness_lufs: None,
    }
}

fn set_default_flags(plan: &mut [AudioPlan]) {
    for (i, p) in plan.iter_mut().enumerate() {
        p.default = i == 0;
    }
}

/// Move one output track to a new position (per-file reordering). The first
/// track always carries the default flag afterwards.
pub fn move_track(plan: &mut Vec<AudioPlan>, from: usize, to: usize) -> Result<()> {
    if from >= plan.len() || to >= plan.len() {
        return Err(Error::Invalid {
            field: "audio track position",
            reason: format!("{from} -> {to} is out of range for {} tracks", plan.len()),
        });
    }
    let item = plan.remove(from);
    plan.insert(to, item);
    set_default_flags(plan);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(index: u32, ord: u32, codec: AudioCodec, ch: u32, lang: &str) -> AudioTrack {
        AudioTrack {
            index,
            pid: None,
            audio_ordinal: ord,
            codec,
            codec_name: String::new(),
            profile: None,
            channels: ch,
            language: Some(lang.into()),
            title: None,
            object_audio: false,
            embedded_core: false,
            default: ord == 0,
            kbps: None,
        }
    }

    #[test]
    fn truehd_with_ac3_core_keeps_both_passthrough() {
        let tracks = vec![
            track(1, 0, AudioCodec::TrueHd, 8, "eng"),
            track(2, 1, AudioCodec::Ac3, 6, "eng"),
            track(3, 2, AudioCodec::Ac3, 6, "fre"),
        ];
        let plan = default_audio_plan(&tracks, &AudioRules::default());
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].source_codec, AudioCodec::TrueHd);
        assert_eq!(plan[0].action, AudioAction::Passthrough);
        assert!(plan[0].default);
        assert_eq!(plan[1].source_index, 2);
        assert_eq!(plan[1].action, AudioAction::Passthrough);
        assert!(!plan[1].default);
    }

    #[test]
    fn dts_hd_ma_without_ac3_gets_encoded_ac3() {
        let tracks = vec![
            track(1, 0, AudioCodec::Dts, 6, "eng"),
            track(2, 1, AudioCodec::DtsHdMa, 8, "eng"),
        ];
        let plan = default_audio_plan(&tracks, &AudioRules::default());
        assert_eq!(
            plan[0].source_codec,
            AudioCodec::DtsHdMa,
            "lossless must come first"
        );
        assert_eq!(
            plan[1].action,
            AudioAction::Encode {
                target: AudioTarget::Ac3,
                bitrate_kbps: 640,
                channels: 6
            }
        );
        assert_eq!(
            plan[1].source_index, 2,
            "AC-3 is encoded from the lossless track"
        );
    }

    #[test]
    fn stereo_lpcm_dvd_gets_stereo_ac3_bitrate() {
        let tracks = vec![track(1, 0, AudioCodec::Lpcm, 2, "eng")];
        let plan = default_audio_plan(&tracks, &AudioRules::default());
        assert_eq!(
            plan[1].action,
            AudioAction::Encode {
                target: AudioTarget::Ac3,
                bitrate_kbps: 224,
                channels: 2
            }
        );
    }

    #[test]
    fn ac3_only_source_is_single_track() {
        let tracks = vec![track(1, 0, AudioCodec::Ac3, 6, "eng")];
        let plan = default_audio_plan(&tracks, &AudioRules::default());
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].action, AudioAction::Passthrough);
    }

    #[test]
    fn preferred_language_and_additional_language() {
        let tracks = vec![
            track(1, 0, AudioCodec::TrueHd, 8, "jpn"),
            track(2, 1, AudioCodec::DtsHdMa, 6, "eng"),
            track(3, 2, AudioCodec::Ac3, 2, "eng"),
        ];
        let rules = AudioRules {
            languages: vec!["eng".into(), "jpn".into()],
            ..Default::default()
        };
        let plan = default_audio_plan(&tracks, &rules);
        let order: Vec<u32> = plan.iter().map(|p| p.source_index).collect();
        assert_eq!(order, vec![2, 3, 1]);
        assert_eq!(plan[2].role, AudioRole::Additional);
    }

    #[test]
    fn embedded_ac3_core_is_never_chosen() {
        let mut core = track(2, 1, AudioCodec::Ac3, 6, "eng");
        core.embedded_core = true;
        let tracks = vec![track(1, 0, AudioCodec::TrueHd, 8, "eng"), core];
        let plan = default_audio_plan(&tracks, &AudioRules::default());
        // The core is skipped, so a real AC-3 track is encoded instead.
        assert_eq!(plan.len(), 2);
        assert!(matches!(plan[1].action, AudioAction::Encode { .. }));
        assert_eq!(plan[1].source_index, 1);
    }

    #[test]
    fn user_reorder_moves_default_flag() {
        let tracks = vec![
            track(1, 0, AudioCodec::TrueHd, 8, "eng"),
            track(2, 1, AudioCodec::Ac3, 6, "eng"),
        ];
        let mut plan = default_audio_plan(&tracks, &AudioRules::default());
        move_track(&mut plan, 1, 0).unwrap();
        assert_eq!(plan[0].source_codec, AudioCodec::Ac3);
        assert!(plan[0].default && !plan[1].default);
        assert!(move_track(&mut plan, 0, 5).is_err());
    }

    #[test]
    fn track_specs_and_edits() {
        let src = vec![
            track(1, 0, AudioCodec::TrueHd, 8, "eng"),
            track(2, 1, AudioCodec::DtsHdMa, 6, "fre"),
            track(3, 2, AudioCodec::Ac3, 2, "eng"),
        ];
        let p = plan_from_specs(
            &[
                "1".into(),
                "1:eac3".into(),
                "2:dts_core".into(),
                "1:opus:160:2,norm,delay=-40".into(),
                "1:flac,title=Lossless".into(),
            ],
            &src,
        )
        .unwrap();
        assert_eq!(p[0].action, AudioAction::Passthrough);
        assert!(p[0].default && !p[1].default);
        // E-AC-3 from 7.1: FFmpeg's encoder stops at 5.1, default bitrate.
        assert_eq!(
            p[1].action,
            AudioAction::Encode {
                target: AudioTarget::Eac3,
                bitrate_kbps: 1024,
                channels: 6
            }
        );
        assert_eq!(p[1].title.as_deref(), Some("Dolby Digital Plus 5.1"));
        assert_eq!(p[2].output_codec(), AudioCodec::Dts);
        assert_eq!(p[2].loudness_lufs, None);
        assert_eq!(p[3].loudness_lufs, Some(DEFAULT_LOUDNESS_LUFS));
        assert_eq!(p[3].delay_ms, -40);
        assert_eq!(
            p[3].describe(),
            "Opus 2.0 at 160 kb/s from Dolby TrueHD, normalised to -23 LUFS, delayed -40 ms"
        );
        assert_eq!(p[4].title.as_deref(), Some("Lossless"));

        // The DTS core only exists in DTS-HD; AC-3 cannot carry one.
        assert!(parse_track_spec("3:dts_core", &src).is_err());
        assert!(parse_track_spec("4", &src).is_err());
        assert!(parse_track_spec("1:mp3", &src).is_err());
        assert!(parse_track_spec("1:ac3,loud", &src).is_err());
        // Bitrates are kept within the encoder's range; channels within the source.
        assert_eq!(
            parse_track_spec("3:ac3:9999:6", &src).unwrap().action,
            AudioAction::Encode {
                target: AudioTarget::Ac3,
                bitrate_kbps: 640,
                channels: 2
            }
        );
        // Loudness only applies when the audio is re-encoded.
        let mut plan = vec![parse_track_spec("1", &src).unwrap()];
        let edit = TrackEdit {
            target: None,
            bitrate_kbps: 0,
            channels: 8,
            delay_ms: 0,
            normalise: true,
            title: None,
        };
        edit_track(&mut plan, 0, &src[0], &edit).unwrap();
        assert_eq!(plan[0].loudness_lufs, None);
        add_track(&mut plan, &src[2]);
        remove_track(&mut plan, 0).unwrap();
        assert!(plan[0].default && plan[0].source_codec == AudioCodec::Ac3);
        assert!(remove_track(&mut plan, 5).is_err());
    }
}
