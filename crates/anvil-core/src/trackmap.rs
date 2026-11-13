// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Map ffprobe stream indices to mkvmerge track IDs.
//!
//! The planner refers to tracks by ffprobe index. For Matroska sources that
//! equals mkvmerge's track ID, but not for M2TS: mkvmerge numbers tracks its
//! own way and may split a TrueHD stream's embedded AC-3 core into a track of
//! its own. So for non-Matroska sources the runner identifies the file with
//! `mkvmerge -J` and matches tracks by MPEG-TS PID and codec, falling back to
//! order within each track type.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::model::{AudioCodec, SourceInfo, SubtitleCodec};
use crate::{Error, Result};

/// ffprobe stream index -> mkvmerge track ID.
pub type TrackIdMap = BTreeMap<u32, u32>;

pub fn mkvmerge_identify_args(path: &str) -> Vec<String> {
    vec!["-J".into(), path.into()]
}

#[derive(Debug, Deserialize)]
struct Identify {
    #[serde(default)]
    tracks: Vec<MkvTrack>,
}

#[derive(Debug, Clone, Deserialize)]
struct MkvTrack {
    id: u32,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    codec: String,
    #[serde(default)]
    properties: MkvProps,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct MkvProps {
    /// For MPEG-TS this is the PID.
    number: Option<u32>,
}

/// A coarse codec family, enough to tell TrueHD from its AC-3 core.
fn family_of_mkv(codec: &str) -> &'static str {
    let c = codec.to_ascii_lowercase();
    if c.contains("truehd") {
        "truehd"
    } else if c.contains("e-ac-3") || c.contains("eac3") {
        "eac3"
    } else if c.contains("ac-3") || c.contains("ac3") {
        "ac3"
    } else if c.contains("dts") {
        "dts"
    } else if c.contains("pcm") {
        "pcm"
    } else if c.contains("flac") {
        "flac"
    } else if c.contains("pgs") {
        "pgs"
    } else {
        "other"
    }
}

fn family_of_audio(c: AudioCodec) -> &'static str {
    match c {
        AudioCodec::TrueHd => "truehd",
        AudioCodec::Eac3 => "eac3",
        AudioCodec::Ac3 => "ac3",
        AudioCodec::Dts | AudioCodec::DtsHdMa | AudioCodec::DtsHdHra => "dts",
        AudioCodec::Lpcm => "pcm",
        AudioCodec::Flac => "flac",
        _ => "other",
    }
}

/// Build the map from `mkvmerge -J` output.
pub fn map_tracks(source: &SourceInfo, mkvmerge_json: &str) -> Result<TrackIdMap> {
    let ident: Identify = serde_json::from_str(mkvmerge_json)?;
    let mut map = TrackIdMap::new();
    let mut used: Vec<u32> = Vec::new();

    // (ffprobe index, pid, mkvmerge type, codec family) for every stream we use.
    let mut wanted: Vec<(u32, Option<u32>, &str, &str)> =
        vec![(source.video.index, source.video.pid, "video", "")];
    // Embedded AC-3 cores have no mkvmerge track of their own; never used.
    wanted.extend(
        source
            .audio
            .iter()
            .filter(|a| !a.embedded_core)
            .map(|a| (a.index, a.pid, "audio", family_of_audio(a.codec))),
    );
    wanted.extend(source.subtitles.iter().map(|s| {
        let fam = if s.codec == SubtitleCodec::Pgs {
            "pgs"
        } else {
            ""
        };
        (s.index, s.pid, "subtitles", fam)
    }));

    // Pass 1: PID (+ codec family when several tracks share a PID).
    for &(index, pid, kind, fam) in &wanted {
        let Some(pid) = pid else { continue };
        let candidates: Vec<&MkvTrack> = ident
            .tracks
            .iter()
            .filter(|t| t.kind == kind && t.properties.number == Some(pid) && !used.contains(&t.id))
            .collect();
        let pick = match candidates.len() {
            0 => None,
            1 => Some(candidates[0]),
            _ => candidates
                .iter()
                .copied()
                .find(|t| fam.is_empty() || family_of_mkv(&t.codec) == fam)
                .or(Some(candidates[0])),
        };
        if let Some(t) = pick {
            map.insert(index, t.id);
            used.push(t.id);
        }
    }

    // Pass 2: order within each type, for anything still unmatched.
    for kind in ["video", "audio", "subtitles"] {
        // Snapshot the unused tracks of this type, in mkvmerge order.
        let snapshot: Vec<&MkvTrack> = ident
            .tracks
            .iter()
            .filter(|t| t.kind == kind && !used.contains(&t.id))
            .collect();
        let mut free = snapshot.into_iter();
        for &(index, _, k, fam) in wanted.iter().filter(|w| w.2 == kind) {
            if map.contains_key(&index) || k != kind {
                continue;
            }
            // Skip split-out cores (an AC-3 track right after its TrueHD).
            let next = free
                .by_ref()
                .find(|t| fam.is_empty() || family_of_mkv(&t.codec) == fam || kind != "audio");
            match next {
                Some(t) => {
                    map.insert(index, t.id);
                    used.push(t.id);
                }
                None => {
                    return Err(Error::Tool {
                        tool: "mkvmerge".into(),
                        message: format!(
                            "could not match {kind} stream {index} to a mkvmerge track"
                        ),
                    })
                }
            }
        }
    }
    Ok(map)
}

/// For non-Matroska sources, run `mkvmerge -J` and build the map. Matroska
/// sources need no map (IDs already match), so this returns `Ok(None)`.
pub fn identify(source: &SourceInfo, timeout: std::time::Duration) -> Result<Option<TrackIdMap>> {
    if source.container.contains("matroska") {
        return Ok(None);
    }
    let out = crate::process::run_with_timeout(
        "mkvmerge",
        &mkvmerge_identify_args(&source.path),
        timeout,
    )?;
    if !out.success {
        return Err(Error::Tool {
            tool: "mkvmerge".into(),
            message: out
                .stdout
                .lines()
                .chain(out.stderr.lines())
                .last()
                .unwrap_or("identify failed")
                .to_string(),
        });
    }
    map_tracks(source, &out.stdout).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn audio(index: u32, pid: u32, codec: AudioCodec) -> AudioTrack {
        AudioTrack {
            index,
            pid: Some(pid),
            audio_ordinal: index - 1,
            codec,
            codec_name: String::new(),
            profile: None,
            channels: 6,
            language: Some("eng".into()),
            title: None,
            object_audio: false,
            embedded_core: false,
            default: false,
            kbps: None,
        }
    }

    fn m2ts_source() -> SourceInfo {
        let json = include_str!("../tests/fixtures/uhd_dovi_p7_truehd.json");
        let mut s = crate::probe::parse_ffprobe_json(json).unwrap();
        s.container = "mpegts".into();
        s.video.pid = Some(0x1011);
        s.audio = vec![
            audio(1, 0x1100, AudioCodec::TrueHd),
            audio(2, 0x1101, AudioCodec::DtsHdMa),
        ];
        s.subtitles = vec![SubtitleTrack {
            index: 3,
            pid: Some(0x1200),
            codec: SubtitleCodec::Pgs,
            language: Some("eng".into()),
            title: None,
            forced: false,
            default: false,
            kbps: None,
        }];
        s
    }

    // mkvmerge splits TrueHD's AC-3 core into its own track (id 2), which
    // shifts every later ID relative to ffprobe's numbering.
    const IDENTIFY: &str = r#"{"container":{"type":"MPEG transport stream"},"tracks":[
        {"id":0,"type":"video","codec":"HEVC/H.265/MPEG-H","properties":{"number":4113}},
        {"id":1,"type":"audio","codec":"TrueHD Atmos","properties":{"number":4352}},
        {"id":2,"type":"audio","codec":"AC-3","properties":{"number":4352}},
        {"id":3,"type":"audio","codec":"DTS-HD Master Audio","properties":{"number":4353}},
        {"id":4,"type":"subtitles","codec":"HDMV PGS","properties":{"number":4608}}]}"#;

    #[test]
    fn pid_matching_skips_split_ac3_core() {
        let map = map_tracks(&m2ts_source(), IDENTIFY).unwrap();
        assert_eq!(map.get(&0), Some(&0));
        assert_eq!(map.get(&1), Some(&1), "TrueHD, not its AC-3 core");
        assert_eq!(
            map.get(&2),
            Some(&3),
            "DTS-HD MA shifts past the split core"
        );
        assert_eq!(map.get(&3), Some(&4));
    }

    /// Real ffprobe/mkvmerge output for an M2TS made from the gate source:
    /// ffprobe lists TrueHD's AC-3 core (same PID) as stream 2; mkvmerge
    /// does not.
    #[test]
    fn real_m2ts_with_embedded_core() {
        let mut s = m2ts_source();
        s.audio = vec![
            audio(1, 0x1100, AudioCodec::TrueHd),
            audio(2, 0x1100, AudioCodec::Ac3),
            audio(3, 0x1101, AudioCodec::Ac3),
        ];
        s.audio[1].embedded_core = true;
        s.subtitles.clear();
        let ident = r#"{"tracks":[
            {"id":0,"type":"video","codec":"HEVC/H.265/MPEG-H","properties":{"number":4113}},
            {"id":1,"type":"audio","codec":"TrueHD","properties":{"number":4352}},
            {"id":2,"type":"audio","codec":"AC-3","properties":{"number":4353}}]}"#;
        let map = map_tracks(&s, ident).unwrap();
        assert_eq!(map.get(&1), Some(&1));
        assert_eq!(map.get(&2), None, "embedded core has no track");
        assert_eq!(map.get(&3), Some(&2));
    }

    #[test]
    fn order_fallback_without_pids() {
        let mut s = m2ts_source();
        s.video.pid = None;
        s.audio.iter_mut().for_each(|a| a.pid = None);
        s.subtitles.iter_mut().for_each(|t| t.pid = None);
        let map = map_tracks(&s, IDENTIFY).unwrap();
        assert_eq!(map.get(&1), Some(&1));
        assert_eq!(map.get(&2), Some(&3), "family match skips the AC-3 core");
        assert_eq!(map.get(&3), Some(&4));
    }
}
