// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! The QML-facing job editor.
//!
//! Exposes `JobController` to QML. It holds no encoding logic: decisions are
//! delegated to `anvil-core`. Anything that runs an external tool
//! (probing, auto-crop, preview frames, tool detection, batch adding) runs
//! on a background thread with a timeout, and results are queued back onto
//! the Qt event loop, so the window never freezes however large the file.
//!
//! Jobs go to the shared persistent queue (see `queue_bridge`).
//!
//! Lists are passed to QML as newline-separated rows with tab-separated
//! fields, which keeps the bridge simple.

// QML invokables take plain values, so one with eight (audioedit) is
// clearer than a packed string.
#![allow(clippy::too_many_arguments)]

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, status)]
        /// Last error, shown as a red banner. Empty when all is well.
        #[qproperty(QString, error)]
        #[qproperty(QString, source)]
        #[qproperty(QString, summary)]
        #[qproperty(QString, badges)]
        #[qproperty(QString, audio)]
        #[qproperty(QString, warnings)]
        #[qproperty(QString, steps)]
        /// Filter chain labels for the current job, one per line.
        #[qproperty(QString, filters)]
        /// Generated VapourSynth script for the current job.
        #[qproperty(QString, script)]
        /// One preset per line: "id<TAB>name<TAB>description<TAB>origin".
        #[qproperty(QString, presets)]
        #[qproperty(QString, preset)]
        #[qproperty(QString, crop)]
        /// Tool rows: "name<TAB>state<TAB>version<TAB>purpose<TAB>path"; state is ok, missing or optional.
        #[qproperty(QString, tools)]
        /// Hardware encoders, one per line: name, usable (1/0), summary.
        #[qproperty(QString, hardware)]
        #[qproperty(bool, busy)]
        #[qproperty(bool, loaded)]
        #[qproperty(bool, scanning)]
        /// Preview frame as a file:// URL (empty until the first frame).
        #[qproperty(QString, frame)]
        /// Where the frame is from: "0:42:10 · source" or "... · after filters".
        #[qproperty(QString, frameinfo)]
        #[qproperty(bool, framebusy)]
        /// The frame shown is the untouched source (crop drawn over it).
        #[qproperty(bool, framesource)]
        /// Source picture size and current crop, in source pixels.
        #[qproperty(i32, srcw)]
        #[qproperty(i32, srch)]
        #[qproperty(i32, cropl)]
        #[qproperty(i32, cropr)]
        #[qproperty(i32, cropt)]
        #[qproperty(i32, cropb)]
        #[qproperty(f64, duration)]
        #[qproperty(bool, hdr)]
        /// Where the current source came from, when it is not a plain file
        /// (a disc's main title). Empty otherwise.
        #[qproperty(QString, origin)]
        /// A disc title is being copied to MKV (the drop zone offers Cancel).
        #[qproperty(bool, copying)]
        /// Source audio tracks that can be added: "number<TAB>description".
        #[qproperty(QString, audiosources)]
        /// Output subtitle tracks: "description<TAB>language<TAB>title<TAB>
        /// default<TAB>forced<TAB>forced only<TAB>can be forced only<TAB>
        /// can OCR<TAB>OCR<TAB>kind".
        #[qproperty(QString, subs)]
        /// Source subtitle tracks that can be added: "number<TAB>description".
        #[qproperty(QString, subsources)]
        /// Chapter rows "time<TAB>title" and the mode: keep, none or custom.
        #[qproperty(QString, chapters)]
        #[qproperty(QString, chaptermode)]
        #[qproperty(QString, chaptersummary)]
        /// Titles of the disc the source came from:
        /// "id<TAB>description<TAB>main<TAB>open now". Empty for plain files.
        #[qproperty(QString, titles)]
        /// Filter editor: chain rows "id<TAB>label<TAB>setting", the blocks
        /// that can be added "id<TAB>label<TAB>description", what the chain
        /// needs "name<TAB>installed (1, 0 or ?)<TAB>install hint", whether
        /// the script is generated or hand-edited, and the generated script.
        #[qproperty(QString, chain)]
        #[qproperty(QString, catalogue)]
        #[qproperty(QString, plugins)]
        #[qproperty(QString, vsversion)]
        #[qproperty(QString, scriptmode)]
        #[qproperty(QString, generated)]
        #[qproperty(QString, filtersource)]
        /// Preset rules that matched the open file, one per line.
        #[qproperty(QString, rules)]
        /// The speed estimate, or its progress ("" before one is asked for).
        #[qproperty(QString, estimate)]
        #[qproperty(bool, estimating)]
        type JobController = super::JobControllerRust;

        /// Open a file (path, `file://` URL or `~/...`). A folder adds every
        /// video file inside it to the queue instead.
        #[qinvokable]
        fn open(self: Pin<&mut JobController>, path: &QString);

        /// Load a saved ffprobe JSON file instead of probing (UI work without
        /// the original media).
        #[qinvokable]
        fn load(self: Pin<&mut JobController>, json_path: &QString);

        /// Add every MKV / M2TS / MP4 in a folder to the queue with the
        /// current preset, auto-cropping each.
        #[qinvokable]
        fn batch(self: Pin<&mut JobController>, folder: &QString);

        /// Switch preset by id and re-plan.
        #[qinvokable]
        fn choose(self: Pin<&mut JobController>, id: &QString);

        /// Re-read the preset folder (after the editor saves or deletes),
        /// then switch to `id` (empty keeps the current preset).
        #[qinvokable]
        fn reloadpresets(self: Pin<&mut JobController>, id: &QString);

        /// Time a few short encodes and estimate the whole job.
        #[qinvokable]
        fn estimatetime(self: Pin<&mut JobController>);
        #[qinvokable]
        fn estimatestop(self: Pin<&mut JobController>);

        /// Run FFmpeg auto-crop on the loaded source and re-plan.
        #[qinvokable]
        fn autocrop(self: Pin<&mut JobController>);

        /// Set the crop by hand (source pixels; rounded down to even).
        #[qinvokable]
        fn setcrop(self: Pin<&mut JobController>, left: i32, right: i32, top: i32, bottom: i32);

        /// Render a preview frame at a 0..1 position. `filtered` renders the
        /// exact frame from the job's VapourSynth script.
        #[qinvokable]
        fn preview(self: Pin<&mut JobController>, position: f64, filtered: bool);

        /// Move an output audio track (per-file reordering) and re-plan.
        #[qinvokable]
        fn reorder(self: Pin<&mut JobController>, from: i32, to: i32);

        /// Add the current job to the queue. Returns false if it could not.
        #[qinvokable]
        fn enqueue(self: Pin<&mut JobController>) -> bool;

        /// Detect installed tools (runs in the background).
        #[qinvokable]
        fn scan(self: Pin<&mut JobController>);

        /// A preset fully resolved, as TOML, for the Presets page.
        #[qinvokable]
        fn describe(self: &JobController, id: &QString) -> QString;

        /// Clear the error banner.
        #[qinvokable]
        fn dismiss(self: Pin<&mut JobController>);

        /// Stop copying a disc title.
        #[qinvokable]
        fn stop(self: Pin<&mut JobController>);

        /// Filter chain editing.
        #[qinvokable]
        fn filteradd(self: Pin<&mut JobController>, id: &QString);
        #[qinvokable]
        fn filterremove(self: Pin<&mut JobController>, index: i32);
        #[qinvokable]
        fn filtermove(self: Pin<&mut JobController>, from: i32, to: i32);
        /// Change a block setting: strength, preset, width, height, label, code.
        #[qinvokable]
        fn filterset(self: Pin<&mut JobController>, index: i32, key: &QString, value: &QString);
        /// bestsource or lsmash.
        #[qinvokable]
        fn setsource(self: Pin<&mut JobController>, id: &QString);
        /// Use a hand-edited script instead of the generated one.
        #[qinvokable]
        fn scriptuse(self: Pin<&mut JobController>, text: &QString);
        /// Back to the generated script.
        #[qinvokable]
        fn scriptreset(self: Pin<&mut JobController>);
        /// Check which VapourSynth plugins are installed.
        #[qinvokable]
        fn pluginscan(self: Pin<&mut JobController>);

        /// Copy and open another title of the current disc.
        #[qinvokable]
        fn opentitle(self: Pin<&mut JobController>, id: &QString);

        /// Change output audio track `index`: target is copy, ac3, eac3, aac,
        /// opus, flac or dts_core; 0 bitrate picks a sensible default.
        #[qinvokable]
        fn audioedit(
            self: Pin<&mut JobController>,
            index: i32,
            target: &QString,
            bitrate: i32,
            channels: i32,
            delay: i32,
            normalise: bool,
            title: &QString,
        );
        /// Add source audio track `number` (1 = first), passed through.
        #[qinvokable]
        fn audioadd(self: Pin<&mut JobController>, number: i32);
        #[qinvokable]
        fn audioremove(self: Pin<&mut JobController>, index: i32);

        #[qinvokable]
        fn subedit(
            self: Pin<&mut JobController>,
            index: i32,
            language: &QString,
            title: &QString,
            isdefault: bool,
            forced: bool,
            forcedonly: bool,
            ocr: bool,
        );
        /// Add source subtitle track `number` (1 = first).
        #[qinvokable]
        fn subadd(self: Pin<&mut JobController>, number: i32);
        /// Add a subtitle file (.srt, .ass, .ssa, .sup, .idx).
        #[qinvokable]
        fn subfile(self: Pin<&mut JobController>, path: &QString);
        #[qinvokable]
        fn subremove(self: Pin<&mut JobController>, index: i32);
        #[qinvokable]
        fn submove(self: Pin<&mut JobController>, from: i32, to: i32);

        /// keep, none or custom (custom starts from the source's chapters,
        /// or one every 5 minutes when it has none).
        #[qinvokable]
        fn chapmode(self: Pin<&mut JobController>, mode: &QString);
        #[qinvokable]
        fn chaprename(self: Pin<&mut JobController>, index: i32, title: &QString);
        /// Move a chapter to a typed time ("1:02:03.5").
        #[qinvokable]
        fn chaptime(self: Pin<&mut JobController>, index: i32, time: &QString);
        /// Add a chapter at a 0..1 position of the film.
        #[qinvokable]
        fn chapadd(self: Pin<&mut JobController>, position: f64);
        #[qinvokable]
        fn chapremove(self: Pin<&mut JobController>, index: i32);
        #[qinvokable]
        fn chapevery(self: Pin<&mut JobController>, minutes: i32);
        #[qinvokable]
        fn chaprenumber(self: Pin<&mut JobController>);
        /// Import Matroska XML or OGM chapters.
        #[qinvokable]
        fn chapimport(self: Pin<&mut JobController>, path: &QString);
        /// Export the chapters (.xml: Matroska XML, else OGM text).
        #[qinvokable]
        fn chapexport(self: Pin<&mut JobController>, path: &QString);
    }

    impl cxx_qt::Threading for JobController {}
    impl cxx_qt::Initialize for JobController {}
}

use core::pin::Pin;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anvil_core::audio::{default_audio_plan, move_track, AudioAction, AudioPlan, AudioTarget};
use anvil_core::chapters::{self, ChapterPlan};
use anvil_core::crop::Crop;
use anvil_core::disc;
use anvil_core::jobfile::JobFile;
use anvil_core::model::channel_layout;
use anvil_core::model::{DynamicRange, SourceInfo};
use anvil_core::paths::normalise_input_path;
use anvil_core::pipeline::{default_output_path, default_work_dir, plan_job, JobPlan, PlanOptions};
use anvil_core::preset::{Origin, Preset, PresetStore, DEFAULT_PRESET};
use anvil_core::probe::{parse_ffprobe_json, probe_file};
use anvil_core::process::run_with_timeout;
use anvil_core::progress::format_duration;
use anvil_core::scriptgen::{self, FieldOrder, FilterProfile, SourceFilter};
use anvil_core::subtitles::{self, SubSource, SubtitlePlan};
use anvil_core::tools::detect_all;
use anvil_core::trackmap::TrackIdMap;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;

use crate::queue_bridge::{data_dir, runner};

/// Large UHD remuxes on spinning disks or network shares can take a while to
/// open; anything beyond this is treated as a hang.
const PROBE_TIMEOUT: Duration = Duration::from_secs(120);
/// Each auto-crop sample decodes a handful of frames.
const CROP_SAMPLE_TIMEOUT: Duration = Duration::from_secs(60);
const CROP_SAMPLES: u32 = 12;
/// Source preview frames are a quick seek.
const SOURCE_FRAME_TIMEOUT: Duration = Duration::from_secs(60);
/// The first after-filters frame of a big file waits for indexing.
const FILTERED_FRAME_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Bumped for every preview request; only the newest result is shown.
static PREVIEW_GENERATION: AtomicU64 = AtomicU64::new(0);
/// Set by Cancel while a disc title is being copied.
static DISC_CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
pub struct JobControllerRust {
    status: QString,
    error: QString,
    source: QString,
    summary: QString,
    badges: QString,
    audio: QString,
    warnings: QString,
    steps: QString,
    filters: QString,
    script: QString,
    presets: QString,
    preset: QString,
    crop: QString,
    tools: QString,
    busy: bool,
    loaded: bool,
    scanning: bool,
    frame: QString,
    frameinfo: QString,
    framebusy: bool,
    framesource: bool,
    srcw: i32,
    srch: i32,
    cropl: i32,
    cropr: i32,
    cropt: i32,
    cropb: i32,
    duration: f64,
    hdr: bool,
    origin: QString,
    copying: bool,
    audiosources: QString,
    subs: QString,
    subsources: QString,
    chapters: QString,
    chaptermode: QString,
    chaptersummary: QString,
    titles: QString,
    chain: QString,
    catalogue: QString,
    plugins: QString,
    vsversion: QString,
    scriptmode: QString,
    generated: QString,
    filtersource: QString,
    rules: QString,
    hardware: QString,
    estimate: QString,
    estimating: bool,
    estimate_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,

    // Private state, not exposed to QML.
    info: Option<SourceInfo>,
    crop_value: Option<Crop>,
    audio_plan: Option<Vec<AudioPlan>>,
    sub_plan: Option<Vec<SubtitlePlan>>,
    chapter_plan: Option<ChapterPlan>,
    filter_plan: Option<FilterProfile>,
    /// VapourSynth plugins found by the last scan.
    inventory: Option<Result<scriptgen::Inventory, String>>,
    preset_id: String,
    plan: Option<JobPlan>,
    /// mkvmerge track IDs for non-MKV sources (M2TS).
    track_ids: Option<TrackIdMap>,
    /// Tool versions from the last scan, recorded in job files.
    tool_versions: BTreeMap<String, String>,
    /// Origin text and title rows for the next source adopted (set by the
    /// disc flow).
    pending_origin: Option<String>,
    pending_titles: Option<String>,
    /// The disc the current source came from.
    disc_path: Option<String>,
    pending_disc: Option<String>,
}

fn home() -> Option<String> {
    std::env::var("HOME").ok()
}

/// Built-in presets plus the user's own from ~/.config/anvil/presets.
/// The preset for a source, with its rules applied (plain resolve when no
/// source is open).
fn resolve_for(id: &str, info: Option<&SourceInfo>) -> anvil_core::Result<(Preset, Vec<String>)> {
    let s = store()?;
    match info {
        Some(i) => s.resolve_for(id, i, None).map(|r| (r.preset, r.applied)),
        None => s.resolve(id, None).map(|p| (p, Vec::new())),
    }
}

pub(crate) fn user_preset_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| PathBuf::from(h).join(".config")))
        .map(|b| b.join("anvil").join("presets"))
}

pub(crate) fn store() -> anvil_core::Result<PresetStore> {
    let mut s = PresetStore::builtin()?;
    if let Some(dir) = user_preset_dir() {
        s.load_user_dir(&dir)?;
    }
    Ok(s)
}

fn q(s: &str) -> QString {
    QString::from(s)
}

/// Keep rows parseable: tabs and newlines inside a field would split it.
fn field(s: &str) -> String {
    s.replace(['\t', '\n'], " ")
}

fn preview_dir() -> PathBuf {
    std::env::temp_dir().join(format!("anvil-preview-{}", std::process::id()))
}

/// Plan a source with a preset; shared by the editor and batch adding so
/// both make identical jobs.
fn plan_for(
    info: &SourceInfo,
    preset: &Preset,
    crop: Option<Crop>,
    edits: Edits,
    track_ids: Option<TrackIdMap>,
) -> anvil_core::Result<JobPlan> {
    let src = Path::new(&info.path);
    let out_dir = src.parent().unwrap_or(Path::new("."));
    plan_job(
        info,
        preset,
        &PlanOptions {
            work_dir: default_work_dir(src, out_dir),
            output: default_output_path(src, out_dir),
            crop,
            audio_override: edits.audio,
            subtitle_override: edits.subs,
            chapters: edits.chapters,
            filters: edits.filters,
            track_ids,
        },
    )
}

/// Output audio rows for QML: "description<TAB>target<TAB>bitrate<TAB>
/// channels<TAB>delay<TAB>normalise<TAB>source<TAB>title<TAB>max channels<TAB>
/// DTS core possible<TAB>default".
fn audio_rows(plan: &JobPlan, info: &SourceInfo) -> String {
    plan.audio
        .iter()
        .map(|a| {
            let src = info.audio.iter().find(|t| t.index == a.source_index);
            let (target, bitrate, channels) = match &a.action {
                AudioAction::Passthrough => ("copy", 0, src.map_or(2, |t| t.channels)),
                AudioAction::Encode {
                    target,
                    bitrate_kbps,
                    channels,
                } => (target.id(), *bitrate_kbps, *channels),
            };
            let source = src.map_or_else(String::new, |t| {
                format!(
                    "{} {} {}",
                    t.codec.display_name(),
                    channel_layout(t.channels),
                    t.language.as_deref().unwrap_or("und")
                )
            });
            let dts_core = src.is_some_and(|t| {
                matches!(
                    t.codec,
                    anvil_core::model::AudioCodec::DtsHdMa
                        | anvil_core::model::AudioCodec::DtsHdHra
                )
            });
            [
                field(&a.describe()),
                target.into(),
                bitrate.to_string(),
                channels.to_string(),
                a.delay_ms.to_string(),
                u8::from(a.loudness_lufs.is_some()).to_string(),
                field(&source),
                field(a.title.as_deref().unwrap_or("")),
                src.map_or(2, |t| t.channels).to_string(),
                u8::from(dts_core).to_string(),
                u8::from(a.default).to_string(),
            ]
            .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn audio_sources(info: &SourceInfo) -> String {
    info.audio
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.embedded_core)
        .map(|(i, t)| {
            format!(
                "{}\t{}. {} {} {}{}",
                i + 1,
                i + 1,
                t.codec.display_name(),
                channel_layout(t.channels),
                t.language.as_deref().unwrap_or("und"),
                t.title
                    .as_deref()
                    .map(|s| format!(" \"{}\"", field(s)))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn sub_rows(plan: &JobPlan) -> String {
    plan.subtitles
        .iter()
        .map(|s| {
            let can_forced_only = s.codec == anvil_core::model::SubtitleCodec::Pgs
                && matches!(s.source, SubSource::Track { .. });
            [
                field(&s.describe()),
                field(s.language.as_deref().unwrap_or("")),
                field(s.title.as_deref().unwrap_or("")),
                u8::from(s.default).to_string(),
                u8::from(s.forced).to_string(),
                u8::from(s.forced_only).to_string(),
                u8::from(can_forced_only).to_string(),
                u8::from(subtitles::can_ocr(s)).to_string(),
                u8::from(s.ocr).to_string(),
                if matches!(s.source, SubSource::File { .. }) {
                    "file"
                } else {
                    "track"
                }
                .into(),
            ]
            .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn sub_sources(info: &SourceInfo) -> String {
    info.subtitles
        .iter()
        .enumerate()
        .map(|(i, t)| {
            format!(
                "{}\t{}. {} {}{}{}",
                i + 1,
                i + 1,
                subtitles::codec_name(t.codec),
                t.language.as_deref().unwrap_or("und"),
                t.title
                    .as_deref()
                    .map(|s| format!(" \"{}\"", field(s)))
                    .unwrap_or_default(),
                if t.forced { " (forced)" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Chapter rows ("time<TAB>title"), the mode and a one-line summary.
fn chapter_rows(plan: &JobPlan, info: &SourceInfo) -> (String, &'static str, String) {
    let (list, mode) = match &plan.chapters {
        ChapterPlan::Keep => (info.chapters.clone(), "keep"),
        ChapterPlan::Drop => (Vec::new(), "none"),
        ChapterPlan::Custom { chapters } => (chapters.clone(), "custom"),
    };
    let rows = list
        .iter()
        .map(|c| {
            format!(
                "{}\t{}",
                chapters::format_time(c.start_secs),
                field(&c.title)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    (rows, mode, plan.chapters.describe(info.chapters.len()))
}

fn chain_rows(p: &FilterProfile) -> String {
    p.blocks
        .iter()
        .map(|b| format!("{}\t{}\t{}", b.id(), field(&b.label()), field(&b.setting())))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the chain needs and whether it is installed (1, 0, or ? before a scan).
fn plugin_rows(p: &FilterProfile, inv: Option<&Result<scriptgen::Inventory, String>>) -> String {
    if p.blocks.is_empty() && p.script.is_none() {
        // Crop-only jobs decode with FFmpeg: nothing to install.
        return String::new();
    }
    p.requirements()
        .iter()
        .map(|r| {
            let have = match inv {
                Some(Ok(i)) => u8::from(i.has(r)).to_string(),
                _ => "?".into(),
            };
            format!("{}\t{have}\t{}", r.name(), field(r.install_hint()))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The user's per-file changes; `None` keeps the preset's choice.
#[derive(Default)]
struct Edits {
    audio: Option<Vec<AudioPlan>>,
    subs: Option<Vec<SubtitlePlan>>,
    chapters: Option<ChapterPlan>,
    filters: Option<FilterProfile>,
}

/// Make a queue thumbnail in the background and attach it to the job.
fn thumbnail_later(id: i64, info: SourceInfo) {
    std::thread::spawn(move || {
        let dir = data_dir().join("thumbs");
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let out = dir.join(format!("{id}.jpg")).to_string_lossy().into_owned();
        let cmd = anvil_core::preview::thumbnail_command(&info, &out);
        if run_with_timeout(&cmd.program, &cmd.args, SOURCE_FRAME_TIMEOUT).is_ok_and(|o| o.success)
        {
            if let Some(r) = runner() {
                let _ = r.set_thumbnail(id, &out);
            }
        }
    });
}

/// Where a disc title's MKV copy goes: beside the disc folder when that is
/// writable, else ~/Videos/Anvil (discs mounted read-only).
fn copy_dir(d: &disc::Disc) -> PathBuf {
    let beside = d.root.parent().map(Path::to_path_buf);
    if let Some(dir) = beside {
        let probe = dir.join(format!(".anvil-write-test-{}", std::process::id()));
        if std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return dir;
        }
    }
    let fallback = PathBuf::from(home().unwrap_or_else(|| ".".into()))
        .join("Videos")
        .join("Anvil");
    let _ = std::fs::create_dir_all(&fallback);
    fallback
}

/// Video files in a folder (two levels deep), skipping disc structures and
/// Anvil's own work folders.
fn media_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.starts_with('.') || name == "BDMV" || name == "VIDEO_TS" {
            continue;
        }
        if p.is_dir() {
            if depth > 0 {
                media_files(&p, depth - 1, out);
            }
        } else if let Some(ext) = p.extension().map(|e| e.to_ascii_lowercase()) {
            let is_output = name.contains(".anvil.");
            if !is_output && ["mkv", "m2ts", "mp4", "ts"].iter().any(|e| ext == *e) {
                out.push(p);
            }
        }
    }
}

impl cxx_qt::Initialize for qobject::JobController {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().publish_presets();
        self.as_mut().set_preset(q(DEFAULT_PRESET));
        self.as_mut().rust_mut().get_mut().preset_id = DEFAULT_PRESET.to_string();
        self.as_mut()
            .set_status(q("Drop a film or a folder, or browse, to start"));
        self.scan();
    }
}

impl qobject::JobController {
    pub fn open(mut self: Pin<&mut Self>, path: &QString) {
        let path = normalise_input_path(&path.to_string(), home().as_deref());
        if path.is_empty() || self.rust().busy {
            return;
        }
        let p = Path::new(&path);
        if !p.exists() {
            self.as_mut()
                .set_error(q(&format!("File not found: {path}")));
            return;
        }
        if disc::detect(p).is_some() {
            return self.open_disc(path, None);
        }
        if p.is_dir() {
            return self.batch(&q(&path));
        }
        self.as_mut().set_error(QString::default());
        self.as_mut().set_busy(true);
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.as_mut().set_status(q(&format!(
            "Reading {name}… large UHD files can take a moment"
        )));

        let thread = self.qt_thread();
        std::thread::spawn(move || {
            // For M2TS and other non-MKV sources, also map track IDs now so
            // the mux step references the right tracks.
            let result = probe_file(&path, PROBE_TIMEOUT).map(|info| {
                let ids = anvil_core::trackmap::identify(&info, PROBE_TIMEOUT)
                    .ok()
                    .flatten();
                (info, ids)
            });
            let _ = thread.queue(move |mut qobj| {
                qobj.as_mut().set_busy(false);
                match result {
                    Ok((info, ids)) => {
                        qobj.as_mut().rust_mut().get_mut().track_ids = ids;
                        qobj.as_mut().adopt(info, &path)
                    }
                    Err(e) => {
                        qobj.as_mut()
                            .set_error(q(&format!("Could not read {path}: {e}")));
                        qobj.as_mut()
                            .set_status(q("Drop a film or a folder, or browse, to start"));
                    }
                }
            });
        });
    }

    /// Open a Blu-ray or DVD folder: copy its main title to MKV (no
    /// re-encoding, progress in the status line), then open that file.
    fn open_disc(mut self: Pin<&mut Self>, path: String, choice: Option<String>) {
        self.as_mut().set_error(QString::default());
        self.as_mut().set_busy(true);
        self.as_mut().set_copying(true);
        self.as_mut().set_status(q("Reading the disc's titles…"));
        DISC_CANCEL.store(false, Ordering::SeqCst);
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let progress_thread = thread.clone();
            let result = (|| -> Result<(PathBuf, String, String), String> {
                let d = disc::scan(Path::new(&path), PROBE_TIMEOUT).map_err(|e| e.to_string())?;
                let best = disc::main_title(&d)
                    .ok_or("the disc has no titles")?
                    .clone();
                let main = match &choice {
                    Some(id) => d
                        .titles
                        .iter()
                        .find(|t| &t.id == id)
                        .cloned()
                        .ok_or_else(|| format!("the disc has no title {id}"))?,
                    None => best.clone(),
                };
                let rows = d
                    .titles
                    .iter()
                    .map(|t| {
                        format!(
                            "{}\t{}\t{}\t{}",
                            field(&t.id),
                            field(&t.describe(d.kind)),
                            u8::from(t.id == best.id),
                            u8::from(t.id == main.id)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let what = main.describe(d.kind);
                let target = disc::remux_path(&d, &main, &copy_dir(&d));
                let mut origin = format!(
                    "From the {} {}: {what}{}{}. Copied to {} without re-encoding.",
                    d.kind.name(),
                    d.label,
                    if main.id == best.id && d.titles.len() > 1 {
                        " (the main title)"
                    } else {
                        ""
                    },
                    if d.titles.len() > 1 {
                        format!(", one of {} titles", d.titles.len())
                    } else {
                        String::new()
                    },
                    target.display()
                );
                for n in main.notes(d.kind) {
                    origin.push(' ');
                    origin.push_str(&n);
                }
                let mut last = -1i64;
                let (kind, label) = (d.kind.name(), d.label.clone());
                disc::copy_title(&main, &target, &DISC_CANCEL, |p| {
                    let pct = (p * 100.0).round() as i64;
                    if pct != last {
                        last = pct;
                        let note = format!(
                            "Copying the {kind} {label} main title ({what}) to MKV: {pct}%"
                        );
                        let _ = progress_thread
                            .queue(move |mut qobj| qobj.as_mut().set_status(q(&note)));
                    }
                })
                .map_err(|e| e.to_string())?;
                Ok((target, origin, rows))
            })();
            let _ = thread.queue(move |mut qobj| {
                qobj.as_mut().set_copying(false);
                qobj.as_mut().set_busy(false);
                match result {
                    Ok((target, origin, rows)) => {
                        let r = qobj.as_mut().rust_mut().get_mut();
                        r.pending_origin = Some(origin);
                        r.pending_titles = Some(rows);
                        r.pending_disc = Some(path.clone());
                        qobj.as_mut().open(&q(&target.to_string_lossy()));
                    }
                    Err(e) => {
                        if DISC_CANCEL.load(Ordering::SeqCst) {
                            qobj.as_mut().set_status(q("Disc copy cancelled"));
                        } else {
                            qobj.as_mut()
                                .set_error(q(&format!("Could not open the disc: {e}")));
                            qobj.as_mut()
                                .set_status(q("Drop a film or a folder, or browse, to start"));
                        }
                    }
                }
            });
        });
    }

    pub fn stop(self: Pin<&mut Self>) {
        DISC_CANCEL.store(true, Ordering::SeqCst);
    }

    // ---- Filter chain -------------------------------------------------------

    fn current_filters(&self) -> Option<(FilterProfile, SourceInfo)> {
        let r = self.rust();
        Some((r.plan.as_ref()?.profile.clone(), r.info.clone()?))
    }

    fn use_filters(self: Pin<&mut Self>, result: Result<FilterProfile, String>) {
        self.apply(
            result.map_err(|reason| anvil_core::Error::Invalid {
                field: "filters",
                reason,
            }),
            |r, p| r.filter_plan = Some(p),
        );
    }

    pub fn filteradd(self: Pin<&mut Self>, id: &QString) {
        let Some((mut p, info)) = self.current_filters() else {
            return;
        };
        let order = if info.video.top_field_first {
            FieldOrder::TopFieldFirst
        } else {
            FieldOrder::BottomFieldFirst
        };
        let result = match scriptgen::new_block(&id.to_string(), order) {
            Some(b) => {
                // Field filters go first; everything else at the end.
                if b.runs_before_crop() {
                    p.blocks.insert(0, b);
                } else {
                    p.blocks.push(b);
                }
                Ok(p)
            }
            None => Err(format!("no filter called {id}")),
        };
        self.use_filters(result);
    }

    pub fn filterremove(self: Pin<&mut Self>, index: i32) {
        let Some((mut p, _)) = self.current_filters() else {
            return;
        };
        if (index.max(0) as usize) < p.blocks.len() {
            p.blocks.remove(index.max(0) as usize);
            self.use_filters(Ok(p));
        }
    }

    pub fn filtermove(self: Pin<&mut Self>, from: i32, to: i32) {
        let Some((mut p, _)) = self.current_filters() else {
            return;
        };
        let (from, to) = (from.max(0) as usize, to.max(0) as usize);
        if from < p.blocks.len() && to < p.blocks.len() {
            let b = p.blocks.remove(from);
            p.blocks.insert(to, b);
            self.use_filters(Ok(p));
        }
    }

    pub fn filterset(self: Pin<&mut Self>, index: i32, key: &QString, value: &QString) {
        let Some((mut p, _)) = self.current_filters() else {
            return;
        };
        let result = match p.blocks.get_mut(index.max(0) as usize) {
            Some(b) => b.set(&key.to_string(), &value.to_string()).map(|_| p),
            None => return,
        };
        self.use_filters(result);
    }

    pub fn setsource(self: Pin<&mut Self>, id: &QString) {
        let Some((mut p, _)) = self.current_filters() else {
            return;
        };
        p.source = if id.to_string() == "lsmash" {
            SourceFilter::LSmash
        } else {
            SourceFilter::BestSource
        };
        self.use_filters(Ok(p));
    }

    pub fn scriptuse(self: Pin<&mut Self>, text: &QString) {
        let Some((mut p, _)) = self.current_filters() else {
            return;
        };
        let text = text.to_string();
        let result = if text.contains("set_output") {
            p.script = Some(text);
            Ok(p)
        } else {
            Err(
                "the script must end with clip.set_output() so the encoder gets a picture"
                    .to_string(),
            )
        };
        self.use_filters(result);
    }

    pub fn scriptreset(self: Pin<&mut Self>) {
        let Some((mut p, _)) = self.current_filters() else {
            return;
        };
        p.script = None;
        self.use_filters(Ok(p));
    }

    pub fn estimatetime(mut self: Pin<&mut Self>) {
        if self.rust().estimating {
            return;
        }
        let Some(probe) = self
            .rust()
            .plan
            .as_ref()
            .and_then(|p| p.speed_probe.clone())
        else {
            self.as_mut().set_estimate(q(
                "No estimate: the video is copied, or its length or frame rate is unknown.",
            ));
            return;
        };
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.as_mut().rust_mut().estimate_cancel = Some(cancel.clone());
        self.as_mut().set_estimating(true);
        self.as_mut()
            .set_estimate(q("Timing short test encodes on this machine..."));
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let progress_thread = thread.clone();
            let result = anvil_core::estimate::run(&probe, &cancel, |d, n| {
                let _ = progress_thread.queue(move |mut qobj| {
                    qobj.as_mut().set_estimate(q(&format!(
                        "Timing short test encodes on this machine ({d} of {n})..."
                    )));
                });
            });
            let _ = thread.queue(move |mut qobj| {
                qobj.as_mut().set_estimating(false);
                qobj.as_mut().rust_mut().estimate_cancel = None;
                let text = match result {
                    Ok(Some(e)) => e.describe(),
                    Ok(None) => String::new(),
                    Err(e) => format!("Could not estimate: {e}"),
                };
                qobj.as_mut().set_estimate(q(&text));
            });
        });
    }

    pub fn estimatestop(mut self: Pin<&mut Self>) {
        if let Some(c) = self.as_mut().rust_mut().estimate_cancel.take() {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn pluginscan(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let inv = scriptgen::inventory(Duration::from_secs(30));
            let _ = thread.queue(move |mut qobj| {
                let text = match &inv {
                    Ok(i) => format!("VapourSynth {} · {} plugins", i.version, i.plugins.len()),
                    Err(e) => e.clone(),
                };
                qobj.as_mut().set_vsversion(q(&text));
                qobj.as_mut().rust_mut().get_mut().inventory = Some(inv);
                let rows = qobj
                    .rust()
                    .plan
                    .as_ref()
                    .map(|p| plugin_rows(&p.profile, qobj.rust().inventory.as_ref()))
                    .unwrap_or_default();
                qobj.as_mut().set_plugins(q(&rows));
            });
        });
    }

    pub fn opentitle(self: Pin<&mut Self>, id: &QString) {
        if self.rust().busy {
            return;
        }
        if let Some(path) = self.rust().disc_path.clone() {
            self.open_disc(path, Some(id.to_string()));
        }
    }

    // ---- Track and chapter editing -----------------------------------------
    // Each edit starts from what the current plan holds, changes it, keeps
    // it as the job's override and re-plans. Problems show in the banner.

    fn current_audio(&self) -> Option<(Vec<AudioPlan>, SourceInfo)> {
        let r = self.rust();
        Some((r.plan.as_ref()?.audio.clone(), r.info.clone()?))
    }

    fn current_subs(&self) -> Option<(Vec<SubtitlePlan>, SourceInfo)> {
        let r = self.rust();
        Some((r.plan.as_ref()?.subtitles.clone(), r.info.clone()?))
    }

    /// The chapter list to edit: the custom list, or the source's (or one
    /// every 5 minutes) when switching to custom.
    fn current_chapters(&self) -> Option<(Vec<chapters::Chapter>, SourceInfo)> {
        let r = self.rust();
        let info = r.info.clone()?;
        let list = match &r.plan.as_ref()?.chapters {
            ChapterPlan::Custom { chapters } => chapters.clone(),
            _ if !info.chapters.is_empty() => info.chapters.clone(),
            _ => chapters::every(5.0, info.duration_secs.unwrap_or(0.0)).unwrap_or_default(),
        };
        Some((list, info))
    }

    fn apply<T>(
        mut self: Pin<&mut Self>,
        result: anvil_core::Result<T>,
        set: impl FnOnce(&mut JobControllerRust, T),
    ) {
        match result {
            Ok(v) => {
                set(self.as_mut().rust_mut().get_mut(), v);
                self.as_mut().set_error(QString::default());
                self.replan();
            }
            Err(e) => self.set_error(q(&e.to_string())),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn audioedit(
        self: Pin<&mut Self>,
        index: i32,
        target: &QString,
        bitrate: i32,
        channels: i32,
        delay: i32,
        normalise: bool,
        title: &QString,
    ) {
        let Some((mut plan, info)) = self.current_audio() else {
            return;
        };
        let i = index.max(0) as usize;
        let target = target.to_string();
        let result = (|| {
            let src_index =
                plan.get(i)
                    .map(|p| p.source_index)
                    .ok_or(anvil_core::Error::Invalid {
                        field: "audio track",
                        reason: format!("no output track {}", i + 1),
                    })?;
            let source = info
                .audio
                .iter()
                .find(|t| t.index == src_index)
                .cloned()
                .ok_or(anvil_core::Error::Invalid {
                    field: "audio track",
                    reason: "its source track is missing".into(),
                })?;
            let edit = anvil_core::audio::TrackEdit {
                target: if target == "copy" {
                    None
                } else {
                    AudioTarget::from_id(&target)
                },
                bitrate_kbps: bitrate.max(0) as u32,
                channels: channels.max(1) as u32,
                delay_ms: delay,
                normalise,
                title: Some(title.to_string()),
            };
            anvil_core::audio::edit_track(&mut plan, i, &source, &edit)?;
            Ok(plan)
        })();
        self.apply(result, |r, p| r.audio_plan = Some(p));
    }

    pub fn audioadd(self: Pin<&mut Self>, number: i32) {
        let Some((mut plan, info)) = self.current_audio() else {
            return;
        };
        let result = info
            .audio
            .get((number - 1).max(0) as usize)
            .map(|t| {
                anvil_core::audio::add_track(&mut plan, t);
                plan
            })
            .ok_or(anvil_core::Error::Invalid {
                field: "audio track",
                reason: format!("the source has no audio track {number}"),
            });
        self.apply(result, |r, p| r.audio_plan = Some(p));
    }

    pub fn audioremove(self: Pin<&mut Self>, index: i32) {
        let Some((mut plan, _)) = self.current_audio() else {
            return;
        };
        let result =
            anvil_core::audio::remove_track(&mut plan, index.max(0) as usize).map(|_| plan);
        self.apply(result, |r, p| r.audio_plan = Some(p));
    }

    pub fn subedit(
        self: Pin<&mut Self>,
        index: i32,
        language: &QString,
        title: &QString,
        isdefault: bool,
        forced: bool,
        forcedonly: bool,
        ocr: bool,
    ) {
        let Some((mut plan, _)) = self.current_subs() else {
            return;
        };
        let lang = language.to_string();
        let edit = subtitles::SubEdit {
            language: (!lang.trim().is_empty()).then_some(lang),
            title: Some(title.to_string()),
            default: isdefault,
            forced,
            forced_only: forcedonly,
            ocr,
        };
        let result =
            subtitles::edit_subtitle(&mut plan, index.max(0) as usize, &edit).map(|_| plan);
        self.apply(result, |r, p| r.sub_plan = Some(p));
    }

    pub fn subadd(self: Pin<&mut Self>, number: i32) {
        let Some((mut plan, info)) = self.current_subs() else {
            return;
        };
        let result = subtitles::add_source_subtitle(
            &mut plan,
            &info.subtitles,
            (number - 1).max(0) as usize,
        )
        .map(|_| plan);
        self.apply(result, |r, p| r.sub_plan = Some(p));
    }

    pub fn subfile(self: Pin<&mut Self>, path: &QString) {
        let Some((mut plan, _)) = self.current_subs() else {
            return;
        };
        let path = normalise_input_path(&path.to_string(), home().as_deref());
        let result = subtitles::add_file(&mut plan, Path::new(&path)).map(|_| plan);
        self.apply(result, |r, p| r.sub_plan = Some(p));
    }

    pub fn subremove(self: Pin<&mut Self>, index: i32) {
        let Some((mut plan, _)) = self.current_subs() else {
            return;
        };
        let result = subtitles::remove_subtitle(&mut plan, index.max(0) as usize).map(|_| plan);
        self.apply(result, |r, p| r.sub_plan = Some(p));
    }

    pub fn submove(self: Pin<&mut Self>, from: i32, to: i32) {
        let Some((mut plan, _)) = self.current_subs() else {
            return;
        };
        let result = subtitles::move_subtitle(&mut plan, from.max(0) as usize, to.max(0) as usize)
            .map(|_| plan);
        self.apply(result, |r, p| r.sub_plan = Some(p));
    }

    fn use_chapters(self: Pin<&mut Self>, result: anvil_core::Result<Vec<chapters::Chapter>>) {
        self.apply(result, |r, list| {
            r.chapter_plan = Some(ChapterPlan::Custom { chapters: list })
        });
    }

    pub fn chapmode(self: Pin<&mut Self>, mode: &QString) {
        match mode.to_string().as_str() {
            "keep" => self.apply(Ok(()), |r, _| r.chapter_plan = Some(ChapterPlan::Keep)),
            "none" => self.apply(Ok(()), |r, _| r.chapter_plan = Some(ChapterPlan::Drop)),
            _ => {
                let Some((list, _)) = self.current_chapters() else {
                    return;
                };
                self.use_chapters(Ok(list));
            }
        }
    }

    pub fn chaprename(self: Pin<&mut Self>, index: i32, title: &QString) {
        let Some((mut list, _)) = self.current_chapters() else {
            return;
        };
        let title = title.to_string();
        let result = match list.get_mut(index.max(0) as usize) {
            Some(c) if !title.trim().is_empty() => {
                c.title = title.trim().to_string();
                Ok(list)
            }
            Some(_) => Err(anvil_core::Error::Invalid {
                field: "chapter title",
                reason: "a chapter needs a title".into(),
            }),
            None => return,
        };
        self.use_chapters(result);
    }

    pub fn chaptime(self: Pin<&mut Self>, index: i32, time: &QString) {
        let Some((mut list, info)) = self.current_chapters() else {
            return;
        };
        let i = index.max(0) as usize;
        if i >= list.len() {
            return;
        }
        let text = time.to_string();
        let result = match chapters::parse_time(&text) {
            None => Err(anvil_core::Error::Invalid {
                field: "chapter time",
                reason: format!("'{text}' is not a time such as 1:02:03.5"),
            }),
            Some(t) => {
                let old = list.remove(i);
                chapters::add(&mut list, t, Some(old.title), info.duration_secs).map(|_| list)
            }
        };
        self.use_chapters(result);
    }

    pub fn chapadd(self: Pin<&mut Self>, position: f64) {
        let Some((mut list, info)) = self.current_chapters() else {
            return;
        };
        let t = anvil_core::preview::time_at(&info, position);
        let result = chapters::add(&mut list, t.floor(), None, info.duration_secs).map(|_| list);
        self.use_chapters(result);
    }

    pub fn chapremove(self: Pin<&mut Self>, index: i32) {
        let Some((mut list, _)) = self.current_chapters() else {
            return;
        };
        if (index.max(0) as usize) < list.len() {
            list.remove(index.max(0) as usize);
            self.use_chapters(Ok(list));
        }
    }

    pub fn chapevery(self: Pin<&mut Self>, minutes: i32) {
        let Some((_, info)) = self.current_chapters() else {
            return;
        };
        self.use_chapters(chapters::every(
            f64::from(minutes),
            info.duration_secs.unwrap_or(0.0),
        ));
    }

    pub fn chaprenumber(self: Pin<&mut Self>) {
        let Some((mut list, _)) = self.current_chapters() else {
            return;
        };
        chapters::renumber(&mut list);
        self.use_chapters(Ok(list));
    }

    pub fn chapimport(self: Pin<&mut Self>, path: &QString) {
        let Some((_, info)) = self.current_chapters() else {
            return;
        };
        let path = normalise_input_path(&path.to_string(), home().as_deref());
        let result = std::fs::read_to_string(&path)
            .map_err(|e| anvil_core::Error::Io {
                path: path.clone().into(),
                source: e,
            })
            .and_then(|t| chapters::parse(&t))
            .map(|mut list| {
                chapters::tidy(&mut list, info.duration_secs);
                list
            });
        self.use_chapters(result);
    }

    pub fn chapexport(mut self: Pin<&mut Self>, path: &QString) {
        let Some((list, _)) = self.current_chapters() else {
            return;
        };
        let path = normalise_input_path(&path.to_string(), home().as_deref());
        let text = if path.to_ascii_lowercase().ends_with(".xml") {
            chapters::to_matroska_xml(&list, "eng")
        } else {
            chapters::to_ogm(&list)
        };
        match std::fs::write(&path, text) {
            Ok(()) => self
                .as_mut()
                .set_status(q(&format!("Saved {} chapters to {path}", list.len()))),
            Err(e) => self.set_error(q(&format!("Could not save {path}: {e}"))),
        }
    }

    pub fn load(mut self: Pin<&mut Self>, json_path: &QString) {
        let path = normalise_input_path(&json_path.to_string(), home().as_deref());
        let result = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|j| parse_ffprobe_json(&j).map_err(|e| e.to_string()));
        match result {
            Ok(info) => {
                let shown = if info.path.is_empty() {
                    path.clone()
                } else {
                    info.path.clone()
                };
                self.as_mut().rust_mut().get_mut().track_ids = None;
                self.adopt(info, &shown);
            }
            Err(e) => self.set_error(q(&format!("Could not load probe JSON: {e}"))),
        }
    }

    /// Take a freshly probed source as the current job, then auto-crop it
    /// (when the preset asks) and show a first preview frame.
    fn adopt(mut self: Pin<&mut Self>, info: SourceInfo, shown_path: &str) {
        let real_file = Path::new(&info.path).exists();
        {
            let v = &info.video;
            self.as_mut().set_srcw(v.width as i32);
            self.as_mut().set_srch(v.height as i32);
            self.as_mut()
                .set_duration(info.duration_secs.unwrap_or(0.0));
            self.as_mut().set_hdr(v.dynamic_range != DynamicRange::Sdr);
        }
        {
            let r = self.as_mut().rust_mut().get_mut();
            r.info = Some(info);
            r.crop_value = None;
            r.audio_plan = None;
            r.sub_plan = None;
            r.chapter_plan = None;
            r.filter_plan = None;
        }
        self.as_mut().set_frame(QString::default());
        self.as_mut().set_frameinfo(QString::default());
        self.as_mut().set_error(QString::default());
        self.as_mut().set_source(q(shown_path));
        let (origin, titles, disc) = {
            let r = self.as_mut().rust_mut().get_mut();
            (
                r.pending_origin.take(),
                r.pending_titles.take(),
                r.pending_disc.take(),
            )
        };
        self.as_mut().set_origin(q(&origin.unwrap_or_default()));
        self.as_mut().set_titles(q(&titles.unwrap_or_default()));
        self.as_mut().rust_mut().get_mut().disc_path = disc;
        self.as_mut().set_loaded(true);
        self.as_mut().replan();

        if real_file {
            self.as_mut().preview(0.3, false);
            let auto = resolve_for(&self.rust().preset_id, self.rust().info.as_ref())
                .map(|p| p.0.output.auto_crop)
                .unwrap_or(true);
            if auto {
                self.autocrop();
            }
        }
    }

    pub fn batch(mut self: Pin<&mut Self>, folder: &QString) {
        let dir = PathBuf::from(normalise_input_path(&folder.to_string(), home().as_deref()));
        if self.rust().busy {
            return;
        }
        // A disc folder is one film, not a folder of films.
        if disc::detect(&dir).is_some() {
            return self.open_disc(dir.to_string_lossy().into_owned(), None);
        }
        let mut files = Vec::new();
        media_files(&dir, 2, &mut files);
        if files.is_empty() {
            self.as_mut().set_error(q(&format!(
                "No MKV, M2TS or MP4 files found in {}",
                dir.display()
            )));
            return;
        }
        let Some(queue) = runner() else {
            self.as_mut().set_error(q("The queue is not ready yet."));
            return;
        };
        let preset_id = self.rust().preset_id.clone();
        // Check the preset loads; each file then gets its own rules.
        match store().and_then(|s| s.resolve(&preset_id, None)) {
            Ok(_) => {}
            Err(e) => {
                self.as_mut().set_error(q(&e.to_string()));
                return;
            }
        };
        let tools = self.rust().tool_versions.clone();
        self.as_mut().set_error(QString::default());
        self.as_mut().set_busy(true);
        let total = files.len();
        self.as_mut()
            .set_status(q(&format!("Adding {total} files from {}…", dir.display())));

        let thread = self.qt_thread();
        let queue = queue.clone();
        std::thread::spawn(move || {
            let mut added = 0;
            let mut problems: Vec<String> = Vec::new();
            for (i, file) in files.iter().enumerate() {
                let name = file
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let note = format!(
                    "Adding {} of {total}: {name} (reading and detecting black bars)…",
                    i + 1
                );
                let _ = thread.queue(move |mut qobj| qobj.as_mut().set_status(q(&note)));
                let result = probe_file(&file.to_string_lossy(), PROBE_TIMEOUT).and_then(|info| {
                    let preset = resolve_for(&preset_id, Some(&info))?.0;
                    let ids = anvil_core::trackmap::identify(&info, PROBE_TIMEOUT)
                        .ok()
                        .flatten();
                    let crop = if preset.output.auto_crop {
                        let (c, n) = anvil_core::crop::detect(
                            &info,
                            CROP_SAMPLES,
                            CROP_SAMPLE_TIMEOUT,
                            |_, _| {},
                        );
                        (n > 0).then_some(c)
                    } else {
                        None
                    };
                    let plan = plan_for(&info, &preset, crop, Edits::default(), ids)?;
                    let job = JobFile::new(&preset_id, preset.clone(), plan, tools.clone());
                    let id = queue.add(&job)?;
                    thumbnail_later(id, info);
                    Ok(())
                });
                match result {
                    Ok(()) => added += 1,
                    Err(e) => problems.push(format!("{name}: {e}")),
                }
            }
            let _ = thread.queue(move |mut qobj| {
                qobj.as_mut().set_busy(false);
                qobj.as_mut().set_status(q(&format!(
                    "Added {added} of {total} files to the queue{}",
                    if added > 0 {
                        ". Open the Queue to start them."
                    } else {
                        ""
                    }
                )));
                if !problems.is_empty() {
                    qobj.as_mut().set_error(q(&format!(
                        "Some files could not be added: {}",
                        problems.join("; ")
                    )));
                }
            });
        });
    }

    fn publish_presets(mut self: Pin<&mut Self>) {
        match store() {
            Ok(s) => {
                let settings = anvil_core::settings::Settings::load();
                let list = s
                    .ids()
                    .map(|(id, origin)| {
                        let (name, desc, unavailable) = s
                            .resolve(id, None)
                            .map(|p| {
                                let why = anvil_core::settings::encoder_available(
                                    p.video.encoder,
                                    &settings,
                                )
                                .err()
                                .unwrap_or_default();
                                (p.name, p.description, why)
                            })
                            .unwrap_or_else(|_| (id.to_string(), String::new(), String::new()));
                        let origin = if origin == Origin::User {
                            "user"
                        } else {
                            "built-in"
                        };
                        // Fifth field: why this machine cannot use the
                        // preset (empty when it can).
                        format!(
                            "{id}\t{}\t{}\t{origin}\t{}",
                            field(&name),
                            field(&desc),
                            field(&unavailable)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                self.as_mut().set_presets(q(&list));
            }
            Err(e) => self
                .as_mut()
                .set_error(q(&format!("Could not load presets: {e}"))),
        }
    }

    pub fn reloadpresets(mut self: Pin<&mut Self>, id: &QString) {
        self.as_mut().publish_presets();
        let id = id.to_string();
        let exists = store().is_ok_and(|s| s.ids().any(|(i, _)| i == id));
        let current = self.rust().preset_id.clone();
        let still = store().is_ok_and(|s| s.ids().any(|(i, _)| i == current));
        if exists {
            self.choose(&q(&id));
        } else if !still {
            self.choose(&q(DEFAULT_PRESET));
        } else {
            self.replan();
        }
    }

    pub fn choose(mut self: Pin<&mut Self>, id: &QString) {
        let id = id.to_string();
        self.as_mut().set_preset(q(&id));
        self.as_mut().rust_mut().get_mut().preset_id = id;
        self.replan();
    }

    pub fn autocrop(mut self: Pin<&mut Self>) {
        if self.rust().busy {
            return;
        }
        let Some(info) = self.rust().info.clone() else {
            return;
        };
        if !Path::new(&info.path).exists() {
            self.as_mut().set_error(q(
                "Auto-crop needs the original file; this job was loaded from a saved probe.",
            ));
            return;
        }
        self.as_mut().set_busy(true);
        self.as_mut().set_status(q("Detecting black bars…"));
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let progress_thread = thread.clone();
            let (crop, sampled) = anvil_core::crop::detect(
                &info,
                CROP_SAMPLES,
                CROP_SAMPLE_TIMEOUT,
                move |done, total| {
                    let _ = progress_thread.queue(move |mut qobj| {
                        qobj.as_mut().set_status(q(&format!(
                            "Detecting black bars: sample {done} of {total}…"
                        )));
                    });
                },
            );
            let _ = thread.queue(move |mut qobj| {
                qobj.as_mut().set_busy(false);
                if sampled == 0 {
                    qobj.as_mut()
                        .set_error(q("Auto-crop could not read any frames. Is FFmpeg installed? Check the Tools page."));
                    qobj.as_mut().replan();
                    return;
                }
                qobj.as_mut().rust_mut().get_mut().crop_value = Some(crop);
                qobj.as_mut().replan();
            });
        });
    }

    pub fn setcrop(mut self: Pin<&mut Self>, left: i32, right: i32, top: i32, bottom: i32) {
        let Some(info) = self
            .rust()
            .info
            .as_ref()
            .map(|i| (i.video.width, i.video.height))
        else {
            return;
        };
        let c = Crop {
            left: left.max(0) as u32,
            right: right.max(0) as u32,
            top: top.max(0) as u32,
            bottom: bottom.max(0) as u32,
        }
        .to_mod2();
        match c.validate(info.0, info.1) {
            Ok((w, h)) if w >= 64 && h >= 64 => {
                self.as_mut().rust_mut().get_mut().crop_value = Some(c);
                self.replan();
            }
            _ => self.set_error(q("That crop would leave almost no picture.")),
        }
    }

    pub fn preview(mut self: Pin<&mut Self>, position: f64, filtered: bool) {
        let Some(info) = self.rust().info.clone() else {
            return;
        };
        if !Path::new(&info.path).exists() {
            return;
        }
        let script = self.rust().plan.as_ref().and_then(|p| {
            p.steps.iter().find_map(|s| {
                s.file
                    .as_ref()
                    .filter(|f| f.0.extension().is_some_and(|e| e == "vpy"))
                    .map(|f| f.1.clone())
            })
        });
        if filtered && script.is_none() {
            self.as_mut()
                .set_error(q("Video passthrough has no filters to preview."));
            return;
        }
        // Crop-only jobs: the same FFmpeg decode and crop the encoder gets.
        let direct_crop = self
            .rust()
            .plan
            .as_ref()
            .filter(|p| anvil_core::pipeline::decodes_directly(p))
            .map(|p| p.crop);
        let generation = PREVIEW_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        self.as_mut().set_framebusy(true);
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let dir = preview_dir();
            let _ = std::fs::create_dir_all(&dir);
            let out = dir
                .join(format!("frame-{generation}.png"))
                .to_string_lossy()
                .into_owned();
            let seconds = anvil_core::preview::time_at(&info, position);
            let label = format!(
                "{} · {}",
                format_duration(seconds as u64),
                if filtered { "after filters" } else { "source" }
            );
            let range = info.video.dynamic_range;
            let result = if let (true, Some(crop)) = (filtered, direct_crop) {
                let cmd = anvil_core::preview::direct_frame_command(&info, seconds, &crop, &out);
                run_with_timeout(&cmd.program, &cmd.args, SOURCE_FRAME_TIMEOUT)
                    .map_err(|e| e.to_string())
            } else if filtered {
                let vpy = dir.join(format!("preview-{generation}.vpy"));
                std::fs::write(&vpy, script.unwrap_or_default())
                    .map_err(|e| e.to_string())
                    .and_then(|_| {
                        let frame = anvil_core::preview::frame_at(&info, position);
                        let cmd = anvil_core::preview::script_frame_command(
                            &vpy.to_string_lossy(),
                            frame,
                            range,
                            &out,
                        );
                        // The pipe needs a shell; run_with_timeout runs one program.
                        run_with_timeout(
                            "sh",
                            &["-c".into(), cmd.to_shell()],
                            FILTERED_FRAME_TIMEOUT,
                        )
                        .map_err(|e| e.to_string())
                    })
            } else {
                let cmd = anvil_core::preview::source_frame_command(&info, seconds, &out);
                run_with_timeout(&cmd.program, &cmd.args, SOURCE_FRAME_TIMEOUT)
                    .map_err(|e| e.to_string())
            };
            let ok = matches!(&result, Ok(o) if o.success) && Path::new(&out).exists();
            let detail = match &result {
                Ok(o) => o
                    .stderr
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("no frame produced")
                    .to_string(),
                Err(e) => e.clone(),
            };
            let _ = thread.queue(move |mut qobj| {
                // A newer request superseded this one: drop it.
                if PREVIEW_GENERATION.load(Ordering::SeqCst) != generation {
                    let _ = std::fs::remove_file(&out);
                    return;
                }
                qobj.as_mut().set_framebusy(false);
                if ok {
                    let old = qobj.rust().frame.to_string();
                    qobj.as_mut().set_frame(q(&format!("file://{out}")));
                    qobj.as_mut().set_frameinfo(q(&label));
                    qobj.as_mut().set_framesource(!filtered);
                    if let Some(old) = old.strip_prefix("file://") {
                        let _ = std::fs::remove_file(old);
                    }
                } else {
                    qobj.as_mut()
                        .set_error(q(&format!("Preview failed: {detail}")));
                }
            });
        });
    }

    pub fn reorder(mut self: Pin<&mut Self>, from: i32, to: i32) {
        let current = {
            let r = self.rust();
            match (&r.audio_plan, &r.info) {
                (Some(p), _) => p.clone(),
                (None, Some(info)) => match resolve_for(&r.preset_id, Some(info)) {
                    Ok((preset, _)) => default_audio_plan(&info.audio, &preset.audio),
                    Err(_) => return,
                },
                _ => return,
            }
        };
        let mut plan = current;
        if move_track(&mut plan, from.max(0) as usize, to.max(0) as usize).is_ok() {
            self.as_mut().rust_mut().get_mut().audio_plan = Some(plan);
            self.replan();
        }
    }

    pub fn enqueue(mut self: Pin<&mut Self>) -> bool {
        let Some(queue) = runner() else {
            self.as_mut().set_error(q("The queue is not ready yet."));
            return false;
        };
        let (plan, preset_id, tools, info) = {
            let r = self.rust();
            match (&r.plan, &r.info) {
                (Some(p), Some(i)) => (
                    p.clone(),
                    r.preset_id.clone(),
                    r.tool_versions.clone(),
                    i.clone(),
                ),
                _ => return false,
            }
        };
        let preset = match resolve_for(&preset_id, Some(&info)) {
            Ok((p, _)) => p,
            Err(e) => {
                self.as_mut().set_error(q(&e.to_string()));
                return false;
            }
        };
        // An encoder this machine cannot use would only fail later in the
        // queue; say so now, with how to install it.
        if let Some(kind) = plan.encoder {
            let settings = anvil_core::settings::Settings::load();
            if let Err(why) = anvil_core::settings::encoder_available(kind, &settings) {
                let program = anvil_core::settings::encoder_program(kind).to_ascii_lowercase();
                let hint = anvil_core::tools::install_hint(&program)
                    .filter(|_| settings.hardware_encoders || !kind.is_hardware())
                    .map(|h| format!(": {h}"))
                    .unwrap_or_default();
                self.as_mut()
                    .set_error(q(&format!("This preset cannot encode here ({why}){hint}.")));
                return false;
            }
        }
        let title = Path::new(&plan.source)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let job = JobFile::new(&preset_id, preset, plan, tools);
        match queue.add(&job) {
            Ok(id) => {
                thumbnail_later(id, info);
                self.as_mut()
                    .set_status(q(&format!("Added {title} to the queue")));
                true
            }
            Err(e) => {
                self.as_mut()
                    .set_error(q(&format!("Could not add to the queue: {e}")));
                false
            }
        }
    }

    pub fn scan(mut self: Pin<&mut Self>) {
        if self.rust().scanning {
            return;
        }
        anvil_core::settings::forget_probes();
        let catalogue = scriptgen::CATALOGUE
            .iter()
            .map(|(id, label, what)| format!("{id}\t{label}\t{}", field(what)))
            .collect::<Vec<_>>()
            .join("\n");
        self.as_mut().set_catalogue(q(&catalogue));
        self.as_mut().pluginscan();
        self.as_mut().set_scanning(true);
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let found = detect_all(Duration::from_secs(10));
            let rows = found
                .iter()
                .map(|t| {
                    let state = if t.found() {
                        "ok"
                    } else if t.required {
                        "missing"
                    } else {
                        "optional"
                    };
                    format!(
                        "{}\t{state}\t{}\t{}\t{}",
                        t.name,
                        field(t.version.as_deref().unwrap_or(if t.found() {
                            "version unknown"
                        } else {
                            "not installed"
                        })),
                        field(t.purpose),
                        t.path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let versions: BTreeMap<String, String> = found
                .iter()
                .filter_map(|t| t.version.clone().map(|v| (t.name.to_string(), v)))
                .collect();
            let ffprobe_missing = found.iter().any(|t| t.program == "ffprobe" && !t.found());
            // CPU only (the install-time choice, or the Tools page switch):
            // no GPU checks at all.
            let hardware = if anvil_core::settings::Settings::load().hardware_encoders {
                anvil_core::hw::detect("/dev/dri/renderD128", Duration::from_secs(15))
            } else {
                Vec::new()
            };
            let hardware = hardware
                .iter()
                .map(|r| {
                    format!(
                        "{}\t{}\t{}",
                        r.kind.label(),
                        u8::from(r.usable),
                        field(&r.summary())
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let _ = thread.queue(move |mut qobj| {
                qobj.as_mut().set_tools(q(&rows));
                qobj.as_mut().set_hardware(q(&hardware));
                qobj.as_mut().rust_mut().get_mut().tool_versions = versions;
                qobj.as_mut().set_scanning(false);
                if ffprobe_missing {
                    qobj.as_mut().set_error(q(
                        "FFprobe is not installed, so files cannot be read. See the Tools page.",
                    ));
                }
            });
        });
    }

    pub fn describe(&self, id: &QString) -> QString {
        let id = id.to_string();
        match store().and_then(|s| s.resolve(&id, None)) {
            Ok(p) => q(&toml::to_string_pretty(&p).unwrap_or_else(|e| e.to_string())),
            Err(e) => q(&e.to_string()),
        }
    }

    pub fn dismiss(self: Pin<&mut Self>) {
        self.set_error(QString::default());
    }

    /// Rebuild the plan from current state and publish it to QML.
    fn replan(mut self: Pin<&mut Self>) {
        let (info, crop, edits, preset_id, track_ids) = {
            let r = self.rust();
            (
                r.info.clone(),
                r.crop_value,
                Edits {
                    audio: r.audio_plan.clone(),
                    subs: r.sub_plan.clone(),
                    chapters: r.chapter_plan.clone(),
                    filters: r.filter_plan.clone(),
                },
                r.preset_id.clone(),
                r.track_ids.clone(),
            )
        };
        let Some(info) = info else { return };

        let result = resolve_for(&preset_id, Some(&info)).and_then(|(preset, applied)| {
            plan_for(&info, &preset, crop, edits, track_ids).map(|p| (p, preset.name, applied))
        });

        match result {
            Ok((plan, preset_name, applied)) => {
                self.as_mut().set_rules(q(&applied.join("\n")));
                // A new plan makes an earlier estimate stale.
                if !self.rust().estimating {
                    self.as_mut().set_estimate(QString::default());
                }
                let v = &info.video;
                let summary = format!(
                    "{} · {}×{} · {}-bit{}",
                    v.codec.to_uppercase(),
                    v.width,
                    v.height,
                    v.bit_depth,
                    info.duration_secs
                        .map(|d| format!(
                            " · {}h {:02}m",
                            (d / 3600.0) as u32,
                            ((d % 3600.0) / 60.0) as u32
                        ))
                        .unwrap_or_default()
                );
                let audio = audio_rows(&plan, &info);
                let subs = sub_rows(&plan);
                let (chapter_rows, chapter_mode, chapter_summary) = chapter_rows(&plan, &info);
                let steps = plan
                    .steps
                    .iter()
                    .map(|s| match &s.command {
                        Some(c) => format!("{}\t{}", field(&s.label), field(&c.to_shell())),
                        None => format!(
                            "{}\t{}",
                            field(&s.label),
                            s.file
                                .as_ref()
                                .map(|f| format!("writes {}", f.0.display()))
                                .unwrap_or_default()
                        ),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let script = plan
                    .steps
                    .iter()
                    .find_map(|s| {
                        s.file
                            .as_ref()
                            .filter(|f| f.0.extension().is_some_and(|e| e == "vpy"))
                    })
                    .map(|f| f.1.clone())
                    .unwrap_or_else(|| {
                        "# No VapourSynth script: video is passed through untouched.".into()
                    });
                let c = plan.crop;
                let crop_text = if c.is_none() {
                    format!("No crop · {}×{}", plan.output_size.0, plan.output_size.1)
                } else {
                    format!("Cropped to {}×{}", plan.output_size.0, plan.output_size.1)
                };

                self.as_mut().set_summary(q(&summary));
                self.as_mut().set_badges(q(&plan.badges.join("\n")));
                self.as_mut().set_audio(q(&audio));
                self.as_mut().set_subs(q(&subs));
                self.as_mut().set_chapters(q(&chapter_rows));
                self.as_mut().set_chaptermode(q(chapter_mode));
                self.as_mut().set_chaptersummary(q(&chapter_summary));
                self.as_mut().set_audiosources(q(&audio_sources(&info)));
                self.as_mut().set_subsources(q(&sub_sources(&info)));
                let mut warnings = plan.warnings.clone();
                // A GPU preset on a machine where that encoder did not answer.
                if let Some(kind) = plan.encoder.filter(|k| k.is_hardware()) {
                    let hw = self.hardware().to_string();
                    if let Some(row) = hw
                        .lines()
                        .find(|l| l.starts_with(&format!("{}\t", kind.label())))
                    {
                        let mut parts = row.split('\t');
                        let (_, usable, why) =
                            (parts.next(), parts.next(), parts.next().unwrap_or(""));
                        if usable == Some("0") {
                            warnings.insert(0, format!(
                                "This preset uses {}, which is not usable here ({why}). The job will fail; pick a CPU preset or see the Tools page.",
                                kind.label()
                            ));
                        }
                    }
                }
                self.as_mut().set_warnings(q(&warnings.join("\n")));
                self.as_mut().set_steps(q(&steps));
                self.as_mut().set_filters(q(&plan.filters.join("\n")));
                self.as_mut().set_script(q(&script));
                let generated = scriptgen::render_generated(
                    &plan.profile,
                    &scriptgen::ScriptContext {
                        source_path: info.path.clone(),
                        crop: plan.crop,
                        output_bit_depth: 10,
                    },
                );
                self.as_mut().set_generated(q(&generated));
                self.as_mut().set_chain(q(&chain_rows(&plan.profile)));
                self.as_mut()
                    .set_scriptmode(q(if plan.profile.script.is_some() {
                        "custom"
                    } else {
                        "generated"
                    }));
                self.as_mut().set_filtersource(q(match plan.profile.source {
                    SourceFilter::LSmash => "lsmash",
                    _ => "bestsource",
                }));
                let rows = plugin_rows(&plan.profile, self.rust().inventory.as_ref());
                self.as_mut().set_plugins(q(&rows));
                self.as_mut().set_crop(q(&crop_text));
                self.as_mut().set_cropl(c.left as i32);
                self.as_mut().set_cropr(c.right as i32);
                self.as_mut().set_cropt(c.top as i32);
                self.as_mut().set_cropb(c.bottom as i32);
                if !self.rust().busy {
                    self.as_mut()
                        .set_status(q(&format!("Ready · {preset_name}")));
                }
                self.as_mut().rust_mut().get_mut().plan = Some(plan);
            }
            Err(e) => {
                self.as_mut().rust_mut().get_mut().plan = None;
                self.as_mut().set_steps(QString::default());
                self.as_mut()
                    .set_error(q(&format!("Cannot plan this job: {e}")));
            }
        }
    }
}
