// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Execute a [`JobPlan`].
//!
//! - Steps run as soon as their dependencies finish, so audio encodes and
//!   metadata extraction run alongside the video encode.
//! - Piped steps (`vspipe ... | x265 ...`) run both processes; a failure in
//!   either fails the step, and cancelling kills both.
//! - Progress is parsed from each encoder's stderr and sent as [`Event`]s.
//! - Every step writes a log (command line plus output) to `work/logs/`.
//! - The verify step probes the output and checks it against the plan's
//!   expectations; a failed critical check fails the job.
//!
//! Every step is an external process, so plain threads are used rather than
//! an async runtime: the core stays small and the GUI can call it from a
//! background thread.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Proc, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::command::Command;
use crate::pipeline::{JobPlan, Step, StepKind};
use crate::process::run_with_timeout;
use crate::progress::{parse_line, Progress, ProgressKind};
use crate::sizing::{self, QualitySearch};
use crate::verify::{mediainfo_args, mediainfo_checks, verify, Expectations, VerifyReport};
use crate::{Error, Result};

/// Shared flag to stop a running job from another thread (Ctrl+C, a Cancel
/// button). Cloning shares the flag.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Shared flag to pause a running job. While paused no new steps start and
/// running processes are stopped with SIGSTOP; resuming sends SIGCONT, so an
/// encode continues exactly where it left off.
#[derive(Debug, Clone, Default)]
pub struct PauseToken(Arc<AtomicBool>);

impl PauseToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn pause(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn resume(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
    pub fn is_paused(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    StepStarted {
        index: usize,
        label: String,
    },
    StepProgress {
        index: usize,
        progress: Progress,
    },
    StepFinished {
        index: usize,
        label: String,
        ok: bool,
        secs: f64,
    },
    Verified {
        report: VerifyReport,
    },
}

#[derive(Debug, Clone)]
pub struct RunReport {
    pub secs: f64,
    pub verify: Option<VerifyReport>,
    pub log_dir: PathBuf,
}

enum Msg {
    Progress(usize, Progress),
    Finished(usize, std::result::Result<(), String>),
    Verified(VerifyReport),
}

/// Everything a step thread needs, shared read-only.
struct Ctx {
    expected_frames: Option<u64>,
    duration_secs: Option<f64>,
    output: PathBuf,
    expect: Expectations,
    abort: CancelToken,
    cancel: CancelToken,
    pause: PauseToken,
}

impl Ctx {
    fn stopping(&self) -> bool {
        self.abort.is_cancelled() || self.cancel.is_cancelled()
    }
}

/// Run every step of `plan`, calling `on_event` from the calling thread.
pub fn run_plan(
    plan: &JobPlan,
    cancel: &CancelToken,
    on_event: impl FnMut(Event),
) -> Result<RunReport> {
    run_plan_with(plan, cancel, &PauseToken::new(), on_event)
}

/// As [`run_plan`], with a [`PauseToken`] to pause and resume the job.
pub fn run_plan_with(
    plan: &JobPlan,
    cancel: &CancelToken,
    pause: &PauseToken,
    mut on_event: impl FnMut(Event),
) -> Result<RunReport> {
    let start = Instant::now();
    let log_dir = plan.work_dir.join("logs");
    for dir in [plan.work_dir.as_path(), log_dir.as_path()] {
        std::fs::create_dir_all(dir).map_err(|source| Error::Io {
            path: dir.into(),
            source,
        })?;
    }
    if let Some(parent) = plan.output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.into(),
            source,
        })?;
    }

    let ctx = Arc::new(Ctx {
        expected_frames: plan.expected_frames,
        duration_secs: plan.expect.duration_secs,
        output: plan.output.clone(),
        expect: plan.expect.clone(),
        abort: CancelToken::new(),
        cancel: cancel.clone(),
        pause: pause.clone(),
    });

    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Pending,
        Running,
        Done,
    }
    let n = plan.steps.len();
    let mut state = vec![State::Pending; n];
    let mut started = vec![Instant::now(); n];
    let (tx, rx) = mpsc::channel::<Msg>();
    let mut failure: Option<(usize, String)> = None;
    let mut verify_report = None;
    let mut handles = Vec::new();

    loop {
        if cancel.is_cancelled() {
            ctx.abort.cancel();
        }
        // Start everything whose dependencies are done.
        if failure.is_none() && !ctx.stopping() && !ctx.pause.is_paused() {
            for i in 0..n {
                let ready = state[i] == State::Pending
                    && plan.steps[i]
                        .depends_on
                        .iter()
                        .all(|&d| d < n && state[d] == State::Done);
                if ready {
                    state[i] = State::Running;
                    started[i] = Instant::now();
                    on_event(Event::StepStarted {
                        index: i,
                        label: plan.steps[i].label.clone(),
                    });
                    let step = plan.steps[i].clone();
                    let log_path = log_dir.join(format!("{:02}-{}.log", i + 1, slug(&step.label)));
                    let ctx = Arc::clone(&ctx);
                    let tx = tx.clone();
                    handles.push(std::thread::spawn(move || {
                        let result = execute_step(i, &step, &ctx, &log_path, &tx);
                        let _ = tx.send(Msg::Finished(i, result));
                    }));
                }
            }
        }

        let running = state.iter().filter(|s| **s == State::Running).count();
        if running == 0 {
            if state.iter().all(|s| *s == State::Done) || failure.is_some() || ctx.stopping() {
                break;
            }
            return Err(Error::Invalid {
                field: "plan",
                reason: "steps have unsatisfiable dependencies".into(),
            });
        }

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Msg::Progress(i, progress)) => on_event(Event::StepProgress { index: i, progress }),
            Ok(Msg::Verified(report)) => {
                on_event(Event::Verified {
                    report: report.clone(),
                });
                verify_report = Some(report);
            }
            Ok(Msg::Finished(i, result)) => {
                state[i] = State::Done;
                let ok = result.is_ok();
                on_event(Event::StepFinished {
                    index: i,
                    label: plan.steps[i].label.clone(),
                    ok,
                    secs: started[i].elapsed().as_secs_f64(),
                });
                if let Err(message) = result {
                    if failure.is_none() && !cancel.is_cancelled() {
                        failure = Some((i, message));
                    }
                    // Stop everything else still running.
                    ctx.abort.cancel();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    for h in handles {
        let _ = h.join();
    }

    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    if let Some((i, message)) = failure {
        return Err(Error::StepFailed {
            step: plan.steps[i].label.clone(),
            message,
            log: log_dir.join(format!("{:02}-{}.log", i + 1, slug(&plan.steps[i].label))),
        });
    }
    Ok(RunReport {
        secs: start.elapsed().as_secs_f64(),
        verify: verify_report,
        log_dir,
    })
}

/// Delete the large intermediate files (encoded video and audio, RPUs) from
/// a finished job's work folder, keeping the job file, script and logs.
/// Returns the bytes freed.
pub fn clean_intermediates(plan: &JobPlan) -> u64 {
    const BULKY: &[&str] = &[
        "hevc", "h264", "ivf", "ac3", "eac3", "m4a", "opus", "flac", "bin",
    ];
    let mut freed = 0;
    if let Ok(entries) = std::fs::read_dir(&plan.work_dir) {
        for e in entries.flatten() {
            let path = e.path();
            let bulky = path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| BULKY.contains(&x));
            if bulky && path.is_file() {
                freed += e.metadata().map(|m| m.len()).unwrap_or(0);
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    freed
}

fn slug(label: &str) -> String {
    let s: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    s.split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn execute_step(
    index: usize,
    step: &Step,
    ctx: &Ctx,
    log_path: &Path,
    tx: &Sender<Msg>,
) -> std::result::Result<(), String> {
    match step.kind {
        StepKind::WriteFile => {
            let (path, contents) = step.file.as_ref().ok_or("write step has no file")?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            std::fs::write(path, contents).map_err(|e| format!("{}: {e}", path.display()))
        }
        StepKind::Verify => verify_output(ctx, log_path, tx),
        StepKind::ChooseQuality => {
            let q = step.quality.as_ref().ok_or("quality step has no samples")?;
            choose_quality(index, q, ctx, log_path, tx)
        }
        StepKind::AdjustActiveArea => {
            let job = step
                .active_area
                .as_ref()
                .ok_or("active-area step has no files")?;
            let source_l5 = job.rpu.with_extension("l5.json");
            let out = run_with_timeout(
                "dovi_tool",
                &[
                    "export".into(),
                    "-i".into(),
                    job.rpu.to_string_lossy().into_owned(),
                    "-d".into(),
                    format!("level5={}", source_l5.display()),
                ],
                Duration::from_secs(1800),
            )
            .map_err(|e| e.to_string())?;
            if !out.success {
                return Err(format!("dovi_tool export failed: {}", out.stderr.trim()));
            }
            let text = std::fs::read_to_string(&source_l5).unwrap_or_default();
            let l5 = crate::hdrmeta::L5::parse(&text)?;
            let write = |p: &Path, s: &str| {
                std::fs::write(p, s).map_err(|e| format!("{}: {e}", p.display()))
            };
            let note = if l5.is_empty() {
                // No L5 in the source: zero the offsets as before, nothing to check.
                write(
                    &job.edit_json,
                    "{\n  \"active_area\": { \"crop\": true }\n}\n",
                )?;
                let _ = std::fs::remove_file(&job.expected);
                "The source RPU has no L5 (active area) metadata: offsets set to zero for the cropped picture.".to_string()
            } else {
                let cropped = l5.cropped(&job.crop);
                write(&job.edit_json, &cropped.editor_json())?;
                write(&job.expected, &cropped.editor_json())?;
                format!(
                    "Source L5: {}. Crop: left {}, right {}, top {}, bottom {}. Output L5: {}.",
                    l5.describe(),
                    job.crop.left,
                    job.crop.right,
                    job.crop.top,
                    job.crop.bottom,
                    cropped.describe()
                )
            };
            write(log_path, &format!("{note}\n"))
        }
        StepKind::OcrSubtitles => {
            let job = step.ocr.as_ref().ok_or("OCR step has no files")?;
            let work = job.output.with_extension("ocr");
            let lang = crate::ocr::tesseract_language(job.language.as_deref());
            let (srt, n) = crate::ocr::file_to_srt(
                &job.input,
                job.language.as_deref(),
                job.forced_only,
                &work,
                &|| ctx.stopping(),
                |p| {
                    let _ = tx.send(Msg::Progress(
                        index,
                        Progress {
                            fraction: Some(p),
                            ..Default::default()
                        },
                    ));
                },
            )?;
            let _ = std::fs::remove_dir_all(&work);
            if n == 0 {
                return Err(format!(
                    "OCR found no text in {} (Tesseract language {lang})",
                    job.input.display()
                ));
            }
            std::fs::write(&job.output, srt)
                .map_err(|e| format!("{}: {e}", job.output.display()))?;
            std::fs::write(
                log_path,
                format!("Read {n} captions with Tesseract ({lang}).\n"),
            )
            .map_err(|e| e.to_string())
        }
        StepKind::FilterSubtitles => {
            let (input, output) = step
                .forced_subs
                .as_ref()
                .ok_or("subtitle step has no files")?;
            let data = std::fs::read(input).map_err(|e| format!("{}: {e}", input.display()))?;
            let (bytes, report) = crate::subtitles::pgs::forced_only(&data)?;
            std::fs::write(output, bytes).map_err(|e| format!("{}: {e}", output.display()))?;
            let note = if report.forced == 0 {
                format!(
                    "No forced captions among {} captions: the track will be empty.\n",
                    report.captions
                )
            } else {
                format!(
                    "Kept {} forced of {} captions.\n",
                    report.forced, report.captions
                )
            };
            std::fs::write(log_path, note).map_err(|e| e.to_string())
        }
        StepKind::MeasureLoudness => {
            let cmd = step
                .command
                .as_ref()
                .ok_or("loudness step has no command")?;
            let m = step
                .loudness
                .as_ref()
                .ok_or("loudness step has no target")?;
            run_command(index, cmd, ctx, log_path, tx)?;
            let log = std::fs::read_to_string(log_path).map_err(|e| e.to_string())?;
            let (integrated, peak) = parse_ebur128_summary(&log)
                .ok_or("FFmpeg's loudness summary was not found in the log")?;
            let gain =
                crate::pipeline::loudness_gain(integrated, peak, m.target_lufs, m.max_true_peak_db);
            let note = format!(
                "\nMeasured {integrated:.1} LUFS, true peak {peak:.1} dBTP: gain {gain:+.2} dB (target {} LUFS, peak at most {} dBTP)\n",
                m.target_lufs, m.max_true_peak_db
            );
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(log_path) {
                let _ = f.write_all(note.as_bytes());
            }
            std::fs::write(&m.result_file, format!("{gain:.2}\n"))
                .map_err(|e| format!("{}: {e}", m.result_file.display()))
        }
        _ => {
            let mut cmd = step.command.as_ref().ok_or("step has no command")?.clone();
            if let Some(path) = &step.crf_from {
                let crf = std::fs::read_to_string(path)
                    .map_err(|e| format!("no CRF chosen ({}): {e}", path.display()))?;
                cmd = with_value(&cmd, sizing::CRF_PLACEHOLDER, crf.trim());
            }
            for s in &step.inputs {
                let v = std::fs::read_to_string(&s.file).map_err(|e| {
                    format!("missing {} ({}): {e}", s.placeholder, s.file.display())
                })?;
                cmd = with_value(&cmd, &s.placeholder, v.trim());
            }
            run_command(index, &cmd, ctx, log_path, tx)
        }
    }
}

/// Put `value` wherever `placeholder` appears in the arguments, in a
/// pipeline too.
fn with_value(cmd: &Command, placeholder: &str, value: &str) -> Command {
    let mut c = cmd.clone();
    for a in c.args.iter_mut().filter(|a| a.contains(placeholder)) {
        *a = a.replace(placeholder, value);
    }
    c.stdin_from = cmd
        .stdin_from
        .as_ref()
        .map(|up| Box::new(with_value(up, placeholder, value)));
    c
}

/// Integrated loudness and true peak from FFmpeg's ebur128 summary:
/// "    I:         -23.4 LUFS" and "    Peak:       -1.2 dBFS".
pub fn parse_ebur128_summary(log: &str) -> Option<(f64, f64)> {
    let summary = &log[log.rfind("Summary:")?..];
    let value = |key: &str| {
        summary.lines().find_map(|l| {
            let rest = l.trim().strip_prefix(key)?;
            rest.split_whitespace().next()?.parse::<f64>().ok()
        })
    };
    let integrated = value("I:")?;
    // Silence measures as -70 LUFS and a -inf peak: nothing to normalise.
    let peak = value("Peak:").unwrap_or(f64::NEG_INFINITY);
    (integrated > -70.0).then_some((integrated, peak))
}

/// Encode the samples at a CRF, measure the bitrate, adjust, and write the
/// chosen CRF for the encode step. Usually one or two rounds.
fn choose_quality(
    index: usize,
    q: &QualitySearch,
    ctx: &Ctx,
    log_path: &Path,
    tx: &Sender<Msg>,
) -> std::result::Result<(), String> {
    let mut log = File::create(log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    let _ = writeln!(
        log,
        "Size target: video at most {:.0} kb/s. Preset {} {}, never above {}.",
        q.target_kbps,
        q.label,
        sizing::format_crf(q.base_crf),
        sizing::format_crf(q.max_crf)
    );
    let stem = log_path.with_extension("");
    let n = q.samples.len().max(1);
    // Progress assumes two rounds; a film that fits finishes after one.
    let planned = 2 * n;
    // Sample encodes report against their own length, not the film's.
    let sample_ctx = Ctx {
        expected_frames: Some(sizing::SAMPLE_FRAMES),
        duration_secs: None,
        output: ctx.output.clone(),
        expect: ctx.expect.clone(),
        abort: ctx.abort.clone(),
        cancel: ctx.cancel.clone(),
        pause: ctx.pause.clone(),
    };
    let (quiet_tx, _quiet_rx) = mpsc::channel::<Msg>();
    let mut rounds: Vec<(f64, f64)> = Vec::new();
    loop {
        let crf = match sizing::decide(q, &rounds) {
            sizing::Next::Done(c) => {
                let text = sizing::format_crf(c);
                std::fs::write(&q.result_file, format!("{text}\n"))
                    .map_err(|e| format!("{}: {e}", q.result_file.display()))?;
                let _ = writeln!(log, "Chosen {}: {text}", q.label);
                if let Some((_, kbps)) = rounds.last().filter(|(_, k)| *k > q.target_kbps) {
                    let _ = writeln!(
                        log,
                        "The target is out of reach without going past {} {}: expect about {:.0}% of the target size.",
                        q.label,
                        sizing::format_crf(q.max_crf),
                        kbps / q.target_kbps * 100.0
                    );
                }
                let _ = tx.send(Msg::Progress(
                    index,
                    Progress {
                        fraction: Some(1.0),
                        ..Default::default()
                    },
                ));
                return Ok(());
            }
            // `decide` settles after at most two rounds.
            sizing::Next::Try(c) => c,
        };
        let text = sizing::format_crf(crf);
        let mut bytes = 0u64;
        for (k, sample) in q.samples.iter().enumerate() {
            if ctx.stopping() {
                return Err("cancelled".into());
            }
            let done = rounds.len() * n + k;
            let _ = tx.send(Msg::Progress(
                index,
                Progress {
                    fraction: Some((done as f64 / planned as f64).min(0.99)),
                    ..Default::default()
                },
            ));
            let sample_log = PathBuf::from(format!(
                "{}.round{}-sample{:02}.log",
                stem.display(),
                rounds.len() + 1,
                k + 1
            ));
            run_command(
                index,
                &with_value(sample, sizing::CRF_PLACEHOLDER, &text),
                &sample_ctx,
                &sample_log,
                &quiet_tx,
            )?;
            let out = q.sample_outputs.get(k).ok_or("sample output missing")?;
            bytes += std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
            let _ = std::fs::remove_file(out);
        }
        let kbps = bytes as f64 * 8.0 / 1000.0 / q.sample_secs;
        let _ = writeln!(
            log,
            "Round {}: {} {text} gives {kbps:.0} kb/s ({:.0}% of target)",
            rounds.len() + 1,
            q.label,
            kbps / q.target_kbps * 100.0
        );
        rounds.push((crf, kbps));
    }
}

// ---- Running commands ----------------------------------------------------------

type SharedLog = Arc<Mutex<File>>;

/// When a running command last wrote anything (milliseconds since `EPOCH`),
/// for spotting a tool that has stopped responding.
type Activity = Arc<std::sync::atomic::AtomicU64>;

/// A tool silent for this long (not paused) is treated as stuck. Encoders
/// print progress every second or so; the slowest start (a 4K x265 lookahead
/// behind heavy filters) is a few minutes.
/// `ANVIL_STALL_SECS` overrides it (0 turns the check off).
fn stall_after() -> Option<Duration> {
    match std::env::var("ANVIL_STALL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        Some(0) => None,
        Some(s) => Some(Duration::from_secs(s)),
        None => Some(Duration::from_secs(15 * 60)),
    }
}

static EPOCH: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);

fn now_ms() -> u64 {
    EPOCH.elapsed().as_millis() as u64
}

fn spawn(cmd: &Command, stdin: Stdio) -> std::result::Result<Child, String> {
    use std::os::unix::process::CommandExt;
    Proc::new(&cmd.program)
        .args(&cmd.args)
        // Own process group, so cancelling also stops anything the tool
        // starts itself (wrapper scripts, helper processes).
        .process_group(0)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                format!(
                    "{} is not installed (or not on PATH){}",
                    cmd.program,
                    crate::tools::install_hint(&cmd.program)
                        .map(|h| format!(": {h}"))
                        .unwrap_or_default()
                )
            }
            _ => format!("could not start {}: {e}", cmd.program),
        })
}

// Private helper; grouping these into a struct would only move the noise.
#[allow(clippy::too_many_arguments)]
/// Read a stderr stream, splitting on `\r` as well as `\n` (encoders redraw
/// their progress line with `\r`). Progress is throttled; other lines go to
/// the log and the error tail.
fn read_stream(
    mut stream: impl Read + Send + 'static,
    prefix: String,
    kind: ProgressKind,
    index: usize,
    expected: Option<u64>,
    duration: Option<f64>,
    log: SharedLog,
    tx: Sender<Msg>,
    activity: Activity,
) -> std::thread::JoinHandle<VecDeque<String>> {
    std::thread::spawn(move || {
        let mut tail: VecDeque<String> = VecDeque::new();
        let mut buf = [0u8; 8192];
        let mut line = Vec::new();
        let mut last_sent = Instant::now() - Duration::from_secs(1);
        let mut last_logged = Instant::now() - Duration::from_secs(10);
        let mut handle = |raw: &[u8], tail: &mut VecDeque<String>| {
            let text = String::from_utf8_lossy(raw).trim_end().to_string();
            if text.is_empty() {
                return;
            }
            if let Some(p) = parse_line(kind, &text, expected, duration) {
                if last_sent.elapsed() >= Duration::from_millis(250) {
                    let _ = tx.send(Msg::Progress(index, p));
                    last_sent = Instant::now();
                }
                // Keep logs readable: one progress line every 10 seconds.
                if last_logged.elapsed() < Duration::from_secs(10) {
                    return;
                }
                last_logged = Instant::now();
            } else {
                tail.push_back(format!("{prefix}{text}"));
                if tail.len() > 40 {
                    tail.pop_front();
                }
            }
            if let Ok(mut f) = log.lock() {
                let _ = writeln!(f, "{prefix}{text}");
            }
        };
        loop {
            let n = match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            activity.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
            for &b in &buf[..n] {
                if b == b'\n' || b == b'\r' {
                    handle(&line, &mut tail);
                    line.clear();
                } else {
                    line.push(b);
                }
            }
        }
        handle(&line, &mut tail);
        tail
    })
}

/// Kill a child and every process in its group.
fn signal_group(child: &Child, sig: libc::c_int) {
    // SAFETY: as in kill_group, this only signals the child's own group.
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), sig);
    }
}

fn kill_group(child: &mut Child) {
    let pid = child.id() as libc::pid_t;
    // SAFETY: kill(2) with a negative pid signals the process group we
    // created for this child; it has no memory-safety implications.
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = child.kill();
}

fn describe_status(status: &ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(c), _) => format!("exit code {c}"),
        (None, Some(s)) => format!("signal {s}"),
        _ => "unknown status".into(),
    }
}

fn run_command(
    index: usize,
    cmd: &Command,
    ctx: &Ctx,
    log_path: &Path,
    tx: &Sender<Msg>,
) -> std::result::Result<(), String> {
    let mut file = File::create(log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    let _ = writeln!(file, "$ {}\n", cmd.to_shell());
    let log: SharedLog = Arc::new(Mutex::new(file));
    let kind = ProgressKind::for_program(&cmd.program);
    let activity: Activity = Arc::new(std::sync::atomic::AtomicU64::new(now_ms()));

    // (name, child) in pipeline order: upstream first.
    let mut children: Vec<(String, Child)> = Vec::new();
    let mut readers = Vec::new();

    if let Some(up) = &cmd.stdin_from {
        let mut upstream = spawn(up, Stdio::null())?;
        let up_out = upstream.stdout.take().ok_or("no stdout from upstream")?;
        let mut downstream = match spawn(cmd, Stdio::from(up_out)) {
            Ok(c) => c,
            Err(e) => {
                kill_group(&mut upstream);
                let _ = upstream.wait();
                return Err(e);
            }
        };
        if let Some(err) = upstream.stderr.take() {
            let prefix = format!(
                "[{}] ",
                up.program.rsplit('/').next().unwrap_or(&up.program)
            );
            readers.push(read_stream(
                err,
                prefix,
                ProgressKind::None,
                index,
                None,
                None,
                Arc::clone(&log),
                tx.clone(),
                Arc::clone(&activity),
            ));
        }
        if let Some(err) = downstream.stderr.take() {
            readers.push(read_stream(
                err,
                String::new(),
                kind,
                index,
                ctx.expected_frames,
                ctx.duration_secs,
                Arc::clone(&log),
                tx.clone(),
                Arc::clone(&activity),
            ));
        }
        // Some tools (hdr10plus_tool, dovi_tool) report on stdout. Read it:
        // closing it would make them fail with a broken pipe.
        if let Some(out) = downstream.stdout.take() {
            readers.push(read_stream(
                out,
                String::new(),
                ProgressKind::None,
                index,
                None,
                None,
                Arc::clone(&log),
                tx.clone(),
                Arc::clone(&activity),
            ));
        }
        children.push((up.program.clone(), upstream));
        children.push((cmd.program.clone(), downstream));
    } else {
        let mut child = spawn(cmd, Stdio::null())?;
        if let Some(out) = child.stdout.take() {
            // Tools like mkvmerge report on stdout.
            readers.push(read_stream(
                out,
                String::new(),
                ProgressKind::None,
                index,
                None,
                None,
                Arc::clone(&log),
                tx.clone(),
                Arc::clone(&activity),
            ));
        }
        if let Some(err) = child.stderr.take() {
            readers.push(read_stream(
                err,
                String::new(),
                kind,
                index,
                ctx.expected_frames,
                ctx.duration_secs,
                Arc::clone(&log),
                tx.clone(),
                Arc::clone(&activity),
            ));
        }
        children.push((cmd.program.clone(), child));
    }

    let mut statuses: Vec<Option<ExitStatus>> = vec![None; children.len()];
    let mut cancelled = false;
    let mut stalled = false;
    let mut stopped = false;
    loop {
        // Pause and resume the whole process groups.
        let want_pause = ctx.pause.is_paused() && !ctx.stopping();
        if want_pause != stopped {
            let sig = if want_pause {
                libc::SIGSTOP
            } else {
                libc::SIGCONT
            };
            for (_, c) in children.iter() {
                signal_group(c, sig);
            }
            stopped = want_pause;
            // Time spent paused is not silence.
            activity.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
        }
        let silent_ms =
            now_ms().saturating_sub(activity.load(std::sync::atomic::Ordering::Relaxed));
        if !stopped
            && !cancelled
            && !stalled
            && stall_after().is_some_and(|d| silent_ms >= d.as_millis() as u64)
        {
            stalled = true;
            for (_, c) in children.iter_mut() {
                kill_group(c);
            }
        }
        if ctx.stopping() && !cancelled {
            cancelled = true;
            for (_, c) in children.iter_mut() {
                kill_group(c);
            }
        }
        for (i, (_, c)) in children.iter_mut().enumerate() {
            if statuses[i].is_none() {
                if let Ok(Some(s)) = c.try_wait() {
                    statuses[i] = Some(s);
                }
            }
        }
        if statuses.iter().all(Option::is_some) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let tail: Vec<String> = readers
        .into_iter()
        .flat_map(|r| r.join().unwrap_or_default())
        .collect();
    if cancelled {
        return Err("cancelled".into());
    }
    if stalled {
        let last = tail
            .last()
            .cloned()
            .unwrap_or_else(|| "(it printed nothing)".into());
        return Err(format!(
            "{} stopped responding: no output for {} seconds, so it was stopped. Last message: {last}",
            cmd.program,
            stall_after().map_or(0, |d| d.as_secs())
        ));
    }
    let tail_text = tail
        .iter()
        .rev()
        .take(15)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    // Report the downstream (encoder) first: an upstream killed by a broken
    // pipe is a symptom, not the cause.
    for i in (0..children.len()).rev() {
        let status = statuses[i].expect("all exited");
        // mkvmerge exits 1 for warnings with a usable file.
        let ok = status.success() || (children[i].0 == "mkvmerge" && status.code() == Some(1));
        if !ok {
            return Err(format!(
                "{} failed ({})\n{}",
                children[i].0,
                describe_status(&status),
                tail_text
            ));
        }
    }
    Ok(())
}

// ---- Verification ------------------------------------------------------------

fn verify_output(ctx: &Ctx, log_path: &Path, tx: &Sender<Msg>) -> std::result::Result<(), String> {
    let path = ctx.output.to_string_lossy().into_owned();
    let info =
        crate::probe::probe_file(&path, Duration::from_secs(120)).map_err(|e| e.to_string())?;
    let mut report = verify(&ctx.expect, &info);
    if crate::tools::which("mediainfo").is_some() {
        match run_with_timeout(
            "mediainfo",
            &mediainfo_args(&path),
            Duration::from_secs(120),
        ) {
            Ok(out) if out.success => report
                .checks
                .extend(mediainfo_checks(&ctx.expect, &out.stdout)),
            _ => {}
        }
    }
    if let Ok(mut f) = File::create(log_path) {
        for c in &report.checks {
            let _ = writeln!(
                f,
                "[{}] {}: {}",
                if c.ok {
                    "ok"
                } else if c.critical {
                    "FAIL"
                } else {
                    "warn"
                },
                c.name,
                c.detail
            );
        }
    }
    report.checks.extend(dynamic_hdr_checks(ctx));
    let _ = tx.send(Msg::Verified(report.clone()));
    if report.passed() {
        Ok(())
    } else {
        let failed: Vec<String> = report
            .failures()
            .filter(|c| c.critical)
            .map(|c| format!("{}: {}", c.name, c.detail))
            .collect();
        Err(format!(
            "output does not match the plan:\n{}",
            failed.join("\n")
        ))
    }
}

/// Pipe the output's video (as raw HEVC) into a tool.
fn pipe_output_hevc(output: &Path, tool: &str, args: &[String]) -> std::result::Result<(), String> {
    let mut ff = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(output)
        .args([
            "-map",
            "0:v:0",
            "-c:v",
            "copy",
            "-bsf:v",
            "hevc_mp4toannexb",
            "-f",
            "hevc",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let stdout = ff.stdout.take().ok_or("no ffmpeg output")?;
    let status = std::process::Command::new(tool)
        .args(args)
        .stdin(Stdio::from(stdout))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("{tool}: {e}"))?;
    let _ = ff.wait();
    if status.success() {
        Ok(())
    } else {
        Err(format!("{tool} could not read the output ({status})"))
    }
}

/// Checks on the output's own dynamic metadata: the Dolby Vision L5 must
/// match the crop frame by frame, and HDR10+ must cover every frame.
fn dynamic_hdr_checks(ctx: &Ctx) -> Vec<crate::verify::Check> {
    use crate::verify::check;
    let mut checks = Vec::new();
    let tmp = |ext: &str| ctx.output.with_extension(format!("verify.{ext}"));
    if let Some(expected) = ctx.expect.dv_l5.as_ref().filter(|p| p.is_file()) {
        let rpu = tmp("rpu");
        let l5 = tmp("l5.json");
        let result = (|| {
            let want = crate::hdrmeta::L5::parse(
                &std::fs::read_to_string(expected).map_err(|e| e.to_string())?,
            )?;
            pipe_output_hevc(
                &ctx.output,
                "dovi_tool",
                &[
                    "extract-rpu".into(),
                    "-".into(),
                    "-o".into(),
                    rpu.to_string_lossy().into_owned(),
                ],
            )?;
            let out = run_with_timeout(
                "dovi_tool",
                &[
                    "export".into(),
                    "-i".into(),
                    rpu.to_string_lossy().into_owned(),
                    "-d".into(),
                    format!("level5={}", l5.display()),
                ],
                Duration::from_secs(600),
            )
            .map_err(|e| e.to_string())?;
            if !out.success {
                return Err(out.stderr.trim().to_string());
            }
            let got = crate::hdrmeta::L5::parse(
                &std::fs::read_to_string(&l5).map_err(|e| e.to_string())?,
            )?;
            crate::hdrmeta::compare_l5(&want, &got)
        })();
        let _ = std::fs::remove_file(&rpu);
        let _ = std::fs::remove_file(&l5);
        checks.push(match result {
            Ok(detail) => check("Dolby Vision L5 matches the crop", true, true, detail),
            Err(e) => check("Dolby Vision L5 matches the crop", false, true, e),
        });
    }
    if ctx.expect.hdr10_plus {
        let json = tmp("hdr10plus.json");
        let result = (|| {
            pipe_output_hevc(
                &ctx.output,
                "hdr10plus_tool",
                &[
                    "extract".into(),
                    "-".into(),
                    "-o".into(),
                    json.to_string_lossy().into_owned(),
                ],
            )?;
            let (frames, _) = crate::hdrmeta::hdr10plus_summary(
                &std::fs::read_to_string(&json).map_err(|e| e.to_string())?,
            )?;
            let out = run_with_timeout(
                "ffprobe",
                &[
                    "-v".into(),
                    "error".into(),
                    "-select_streams".into(),
                    "v:0".into(),
                    "-count_packets".into(),
                    "-show_entries".into(),
                    "stream=nb_read_packets".into(),
                    "-of".into(),
                    "csv=p=0".into(),
                    ctx.output.to_string_lossy().into_owned(),
                ],
                Duration::from_secs(600),
            )
            .map_err(|e| e.to_string())?;
            // "259," (an empty side-data column can follow the count).
            let video: usize = out
                .stdout
                .trim()
                .split(',')
                .next()
                .and_then(|n| n.trim().parse().ok())
                .ok_or_else(|| {
                    format!("could not count the video frames ({})", out.stdout.trim())
                })?;
            Ok((frames, video))
        })();
        let _ = std::fs::remove_file(&json);
        checks.push(match result {
            Ok((f, v)) => check(
                "HDR10+ covers every frame",
                f == v,
                true,
                format!("{f} frames of HDR10+ metadata for {v} video frames"),
            ),
            Err(e) => check("HDR10+ covers every frame", false, true, e),
        });
    }
    if let Some(src) = ctx.expect.hdr10plus_source.as_ref().filter(|p| p.is_file()) {
        if let Ok((_, windows)) = std::fs::read_to_string(src)
            .map_err(|e| e.to_string())
            .and_then(|t| crate::hdrmeta::hdr10plus_summary(&t))
        {
            checks.push(check(
                "HDR10+ processing windows",
                windows <= 1,
                false,
                if windows <= 1 {
                    "one window (the whole picture), unaffected by the crop".into()
                } else {
                    format!("up to {windows} windows; their positions are not moved for the crop")
                },
            ));
        }
    }
    checks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AudioCodec, DynamicRange};

    fn plan_with(steps: Vec<Step>, work: &Path) -> JobPlan {
        JobPlan {
            source: "src".into(),
            work_dir: work.to_path_buf(),
            output: work.join("out.mkv"),
            encoder: None,
            crop: Default::default(),
            output_size: (0, 0),
            filters: vec![],
            audio: vec![],
            subtitles: vec![],
            chapters: Default::default(),
            profile: Default::default(),
            warnings: vec![],
            badges: vec![],
            expected_frames: Some(100),
            budget: None,
            speed_probe: None,
            expect: Expectations {
                duration_secs: None,
                width: 0,
                height: 0,
                bit_depth: 8,
                video_codec: String::new(),
                dynamic_range: DynamicRange::Sdr,
                mastering_display: false,
                content_light: false,
                dolby_vision_profile: None,
                hdr10_plus: false,
                audio: Vec::<AudioCodec>::new(),
                subtitles: 0,
                chapters: false,
                chapter_count: None,
                max_bytes: None,
                dv_l5: None,
                hdr10plus_source: None,
            },
            steps,
        }
    }

    fn sh(label: &str, script: &str, deps: Vec<usize>) -> Step {
        Step {
            label: label.into(),
            kind: StepKind::EncodeAudio,
            command: Some(Command::new("sh").args(["-c", script])),
            file: None,
            depends_on: deps,
            ..Default::default()
        }
    }

    fn workdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("anvil-runner-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn runs_in_dependency_order_with_parallel_steps() {
        let w = workdir("order");
        let order = w.join("order.txt");
        let o = order.to_string_lossy().to_string();
        let steps = vec![
            sh("A slow", &format!("sleep 0.3; echo A >> '{o}'"), vec![]),
            sh("B fast", &format!("echo B >> '{o}'"), vec![]),
            sh("C after both", &format!("echo C >> '{o}'"), vec![0, 1]),
        ];
        let mut events = Vec::new();
        run_plan(&plan_with(steps, &w), &CancelToken::new(), |e| {
            events.push(e)
        })
        .unwrap();
        // B finishes first because it runs alongside A; C waits for both.
        assert_eq!(std::fs::read_to_string(&order).unwrap(), "B\nA\nC\n");
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::StepFinished { ok: true, .. }))
                .count(),
            3
        );
        assert!(w.join("logs").join("03-c-after-both.log").exists());
    }

    #[test]
    fn parses_progress_through_a_pipe_and_reports_failures() {
        let w = workdir("pipe");
        // "x265" here is a stand-in that prints x265-style progress with \r.
        let bin = w.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("x265");
        std::fs::write(&fake, "#!/bin/sh\ncat > /dev/null\nprintf '50 frames: 25.00 fps, 900.00 kb/s\\r' >&2\nsleep 0.4\nprintf '100 frames: 25.00 fps, 900.00 kb/s\\n' >&2\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cmd = Command::new(fake.to_string_lossy().to_string())
            .piped_from(Command::new("sh").args(["-c", "head -c 1000 /dev/zero"]));
        let step = Step {
            label: "Encode video".into(),
            kind: StepKind::EncodeVideo,
            command: Some(cmd),
            file: None,
            depends_on: vec![],
            ..Default::default()
        };
        let mut progress = Vec::new();
        run_plan(&plan_with(vec![step], &w), &CancelToken::new(), |e| {
            if let Event::StepProgress { progress: p, .. } = e {
                progress.push(p)
            }
        })
        .unwrap();
        assert!(
            progress
                .iter()
                .any(|p| p.frames == Some(50) && p.fraction == Some(0.5)),
            "{progress:?}"
        );

        // A failing upstream (a broken script) fails the step with its message.
        let bad = Command::new("cat").piped_from(
            Command::new("sh").args(["-c", "echo 'Script error: no such plugin' >&2; exit 3"]),
        );
        let step = Step {
            label: "Encode video".into(),
            kind: StepKind::EncodeVideo,
            command: Some(bad),
            file: None,
            depends_on: vec![],
            ..Default::default()
        };
        let err = run_plan(&plan_with(vec![step], &w), &CancelToken::new(), |_| {}).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("exit code 3") && text.contains("no such plugin"),
            "{text}"
        );
    }

    #[test]
    fn pause_stops_progress_and_resume_continues() {
        let w = workdir("pause");
        let counter = w.join("ticks.txt");
        let c = counter.display().to_string();
        // Appends a tick every 100 ms for about 1.5 s of running time.
        let step = sh(
            "Ticker",
            &format!("for i in $(seq 1 15); do echo t >> '{c}'; sleep 0.1; done"),
            vec![],
        );
        let plan = plan_with(vec![step], &w);
        let pause = PauseToken::new();
        let p2 = pause.clone();
        let counter2 = counter.clone();
        let probe = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            p2.pause();
            std::thread::sleep(Duration::from_millis(300));
            let a = std::fs::read_to_string(&counter2)
                .unwrap_or_default()
                .lines()
                .count();
            std::thread::sleep(Duration::from_millis(600));
            let b = std::fs::read_to_string(&counter2)
                .unwrap_or_default()
                .lines()
                .count();
            p2.resume();
            (a, b)
        });
        run_plan_with(&plan, &CancelToken::new(), &pause, |_| {}).unwrap();
        let (a, b) = probe.join().unwrap();
        assert_eq!(a, b, "no ticks while paused");
        assert_eq!(
            std::fs::read_to_string(&counter).unwrap().lines().count(),
            15,
            "resumed and finished"
        );
    }

    #[test]
    fn failure_stops_dependants_and_cancel_kills_running_steps() {
        let w = workdir("fail");
        let marker = w.join("never.txt");
        let steps = vec![
            sh("Breaks", "echo boom >&2; exit 1", vec![]),
            sh(
                "Never runs",
                &format!("touch '{}'", marker.display()),
                vec![0],
            ),
        ];
        let err = run_plan(&plan_with(steps, &w), &CancelToken::new(), |_| {}).unwrap_err();
        assert!(matches!(err, Error::StepFailed { .. }));
        assert!(!marker.exists());

        let cancel = CancelToken::new();
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            c2.cancel();
        });
        let start = Instant::now();
        let err = run_plan(
            &plan_with(vec![sh("Long", "sleep 30", vec![])], &w),
            &cancel,
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, Error::Cancelled));
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "cancel must kill the process"
        );
    }

    #[test]
    fn loudness_summary_and_gain() {
        let log = "[Parsed_ebur128_0 @ 0x1] Summary:\n\n  Integrated loudness:\n    I:         -26.3 LUFS\n    Threshold: -36.4 LUFS\n\n  Loudness range:\n    LRA:         0.0 LU\n\n  True peak:\n    Peak:      -30.1 dBFS\n";
        assert_eq!(parse_ebur128_summary(log), Some((-26.3, -30.1)));
        assert_eq!(
            crate::pipeline::loudness_gain(-26.3, -30.1, -23.0, -1.0),
            3.3
        );
        // A loud, peaky track is held back by its true peak, not the target.
        assert_eq!(
            crate::pipeline::loudness_gain(-30.0, -2.5, -23.0, -1.0),
            1.5
        );
        // Quieter than needed: turned down.
        assert_eq!(
            crate::pipeline::loudness_gain(-18.0, -0.2, -23.0, -1.0),
            -5.0
        );
        assert_eq!(
            parse_ebur128_summary("Summary:\n    I:  -70.0 LUFS\n"),
            None
        );
        assert_eq!(parse_ebur128_summary("no summary"), None);
    }
}
