// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! The persistent job queue and its scheduler.
//!
//! [`QueueStore`] keeps jobs in SQLite (`~/.local/share/anvil/queue.sqlite3`)
//! so the queue survives restarts: a job that was running when the app
//! closed comes back as *interrupted* and can be retried.
//!
//! [`QueueRunner`] runs pending jobs in queue order on a background thread.
//! Jobs are grouped into slots so the machine is used well without
//! oversubscribing it: one CPU encode at a time, one job per hardware encoder
//! family (NVENC, Quick Sync, VCN, VAAPI) alongside it, and one light job
//! (video passthrough) at a time.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::encoder::EncoderKind;
use crate::jobfile::JobFile;
use crate::pipeline::{JobPlan, StepKind};
use crate::progress::Progress;
use crate::runner::{run_plan_with, CancelToken, Event, PauseToken};
use crate::verify::VerifyReport;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Paused,
    Done,
    Failed,
    Cancelled,
    /// Was running when the app closed.
    Interrupted,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Running => "running",
            JobStatus::Paused => "paused",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
            JobStatus::Interrupted => "interrupted",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "running" => JobStatus::Running,
            "paused" => JobStatus::Paused,
            "done" => JobStatus::Done,
            "failed" => JobStatus::Failed,
            "cancelled" => JobStatus::Cancelled,
            "interrupted" => JobStatus::Interrupted,
            _ => JobStatus::Pending,
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, JobStatus::Running | JobStatus::Paused)
    }

    pub fn is_finished(self) -> bool {
        matches!(
            self,
            JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled | JobStatus::Interrupted
        )
    }
}

/// Which resource a job occupies while it runs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Slot {
    Cpu,
    Gpu(String),
    Light,
}

pub fn slot_for(plan: &JobPlan) -> Slot {
    match plan.encoder {
        None => Slot::Light,
        Some(k) if k.is_hardware() => Slot::Gpu(
            match k {
                EncoderKind::NvEncC => "nvenc",
                EncoderKind::QsvEncC => "qsv",
                EncoderKind::VceEncC => "vce",
                _ => "vaapi",
            }
            .into(),
        ),
        Some(_) => Slot::Cpu,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueueRow {
    pub id: i64,
    pub position: i64,
    pub title: String,
    pub preset_name: String,
    pub status: JobStatus,
    /// Error, cancellation reason or completion note.
    pub message: String,
    pub source: String,
    pub output: String,
    pub badges: Vec<String>,
    pub thumbnail: Option<String>,
    pub created: u64,
    pub started: Option<u64>,
    pub finished: Option<u64>,
    pub verify: Option<VerifyReport>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn db_err(e: rusqlite::Error) -> Error {
    Error::Tool {
        tool: "queue database".into(),
        message: e.to_string(),
    }
}

pub struct QueueStore {
    conn: Connection,
}

impl QueueStore {
    /// Open (or create) the queue database, and mark jobs that were running
    /// when the app last closed as interrupted.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| Error::Io {
                path: dir.into(),
                source,
            })?;
        }
        Self::init(Connection::open(path).map_err(db_err)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().map_err(db_err)?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS jobs (
                 id        INTEGER PRIMARY KEY AUTOINCREMENT,
                 position  INTEGER NOT NULL,
                 title     TEXT NOT NULL,
                 preset    TEXT NOT NULL,
                 status    TEXT NOT NULL,
                 message   TEXT NOT NULL DEFAULT '',
                 source    TEXT NOT NULL,
                 output    TEXT NOT NULL,
                 badges    TEXT NOT NULL DEFAULT '',
                 thumbnail TEXT,
                 created   INTEGER NOT NULL,
                 started   INTEGER,
                 finished  INTEGER,
                 verify    TEXT,
                 job       TEXT NOT NULL
             );",
        )
        .map_err(db_err)?;
        let store = QueueStore { conn };
        store
            .conn
            .execute(
                "UPDATE jobs SET status = 'interrupted', message = 'The app closed while this job was running.' WHERE status IN ('running', 'paused')",
                [],
            )
            .map_err(db_err)?;
        Ok(store)
    }

    pub fn add(&self, job: &JobFile) -> Result<i64> {
        let text = toml::to_string(job).map_err(|e| Error::Preset(e.to_string()))?;
        let title = Path::new(&job.plan.source)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| job.plan.source.clone());
        let position: i64 = self
            .conn
            .query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM jobs", [], |r| {
                r.get(0)
            })
            .map_err(db_err)?;
        self.conn
            .execute(
                "INSERT INTO jobs (position, title, preset, status, source, output, badges, created, job)
                 VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6, ?7, ?8)",
                params![
                    position,
                    title,
                    job.preset.name,
                    job.plan.source,
                    job.plan.output.to_string_lossy(),
                    job.plan.badges.join("\n"),
                    now() as i64,
                    text
                ],
            )
            .map_err(db_err)?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list(&self) -> Result<Vec<QueueRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, position, title, preset, status, message, source, output, badges, thumbnail,
                        created, started, finished, verify
                 FROM jobs ORDER BY position",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                let badges: String = r.get(8)?;
                let verify: Option<String> = r.get(13)?;
                Ok(QueueRow {
                    id: r.get(0)?,
                    position: r.get(1)?,
                    title: r.get(2)?,
                    preset_name: r.get(3)?,
                    status: JobStatus::parse(&r.get::<_, String>(4)?),
                    message: r.get(5)?,
                    source: r.get(6)?,
                    output: r.get(7)?,
                    badges: badges
                        .lines()
                        .map(str::to_string)
                        .filter(|s| !s.is_empty())
                        .collect(),
                    thumbnail: r.get(9)?,
                    created: r.get::<_, i64>(10)? as u64,
                    started: r.get::<_, Option<i64>>(11)?.map(|v| v as u64),
                    finished: r.get::<_, Option<i64>>(12)?.map(|v| v as u64),
                    verify: verify.and_then(|v| serde_json::from_str(&v).ok()),
                })
            })
            .map_err(db_err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)
    }

    pub fn job(&self, id: i64) -> Result<JobFile> {
        let text: String = self
            .conn
            .query_row("SELECT job FROM jobs WHERE id = ?1", [id], |r| r.get(0))
            .map_err(db_err)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn status(&self, id: i64) -> Result<Option<JobStatus>> {
        let s: Option<String> = self
            .conn
            .query_row("SELECT status FROM jobs WHERE id = ?1", [id], |r| r.get(0))
            .optional()
            .map_err(db_err)?;
        Ok(s.map(|s| JobStatus::parse(&s)))
    }

    pub fn set_status(&self, id: i64, status: JobStatus, message: &str) -> Result<()> {
        let (started, finished) = match status {
            JobStatus::Running => ("COALESCE(started, ?3)", "NULL"),
            s if s.is_finished() => ("started", "?3"),
            _ => ("started", "finished"),
        };
        let sql = format!(
            "UPDATE jobs SET status = ?1, message = ?2, started = {started}, finished = {finished} WHERE id = ?4"
        );
        self.conn
            .execute(&sql, params![status.as_str(), message, now() as i64, id])
            .map_err(db_err)?;
        Ok(())
    }

    pub fn set_verify(&self, id: i64, report: &VerifyReport) -> Result<()> {
        let json = serde_json::to_string(report)?;
        self.conn
            .execute(
                "UPDATE jobs SET verify = ?1 WHERE id = ?2",
                params![json, id],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn set_thumbnail(&self, id: i64, path: &str) -> Result<()> {
        self.conn
            .execute(
                "UPDATE jobs SET thumbnail = ?1 WHERE id = ?2",
                params![path, id],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Put a finished job back in the queue.
    pub fn retry(&self, id: i64) -> Result<()> {
        self.conn
            .execute(
                "UPDATE jobs SET status = 'pending', message = '', started = NULL, finished = NULL, verify = NULL WHERE id = ?1",
                [id],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Move a job up (`delta < 0`) or down by swapping with its neighbour.
    pub fn move_by(&mut self, id: i64, delta: i64) -> Result<()> {
        let tx = self.conn.transaction().map_err(db_err)?;
        let pos: i64 = tx
            .query_row("SELECT position FROM jobs WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .map_err(db_err)?;
        let sql = if delta < 0 {
            "SELECT id, position FROM jobs WHERE position < ?1 ORDER BY position DESC LIMIT 1"
        } else {
            "SELECT id, position FROM jobs WHERE position > ?1 ORDER BY position ASC LIMIT 1"
        };
        let other: Option<(i64, i64)> = tx
            .query_row(sql, [pos], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(db_err)?;
        if let Some((oid, opos)) = other {
            tx.execute(
                "UPDATE jobs SET position = ?1 WHERE id = ?2",
                params![opos, id],
            )
            .map_err(db_err)?;
            tx.execute(
                "UPDATE jobs SET position = ?1 WHERE id = ?2",
                params![pos, oid],
            )
            .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)
    }

    pub fn remove(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM jobs WHERE id = ?1", [id])
            .map_err(db_err)?;
        Ok(())
    }

    pub fn clear_finished(&self) -> Result<usize> {
        self.conn
            .execute("DELETE FROM jobs WHERE status IN ('done', 'cancelled')", [])
            .map_err(db_err)
    }

    /// The first pending job (in queue order) whose slot is free.
    fn next_runnable(&self, busy: &[Slot]) -> Result<Option<(i64, JobFile)>> {
        for row in self.list()? {
            if row.status != JobStatus::Pending {
                continue;
            }
            let job = match self.job(row.id) {
                Ok(j) => j,
                Err(e) => {
                    self.set_status(
                        row.id,
                        JobStatus::Failed,
                        &format!("Could not read job: {e}"),
                    )?;
                    continue;
                }
            };
            if !busy.contains(&slot_for(&job.plan)) {
                return Ok(Some((row.id, job)));
            }
        }
        Ok(None)
    }
}

/// Live progress of a running job, for the queue view.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LiveProgress {
    /// Whole-job progress, 0.0 to 1.0.
    pub fraction: f64,
    pub step: String,
    pub steps_done: usize,
    pub steps_total: usize,
    /// Progress of the main running step (the video encode when running).
    pub detail: Option<Progress>,
    pub elapsed_secs: u64,
}

#[derive(Debug, Clone)]
pub enum QueueEvent {
    /// Statuses or the job list changed: reload rows.
    Changed,
    /// Live progress moved for a running job.
    Progress(i64),
}

struct Running {
    slot: Slot,
    cancel: CancelToken,
    pause: PauseToken,
    progress: LiveProgress,
}

struct Shared {
    store: Mutex<QueueStore>,
    state: Mutex<State>,
    wake: Condvar,
    on_event: Box<dyn Fn(QueueEvent) + Send + Sync>,
}

struct State {
    held: bool,
    shutdown: bool,
    running: HashMap<i64, Running>,
}

/// Runs queued jobs on background threads. Cheap to clone; clones share
/// the same queue.
#[derive(Clone)]
pub struct QueueRunner {
    shared: Arc<Shared>,
}

/// Relative cost of each step kind for whole-job progress.
fn weight(kind: &StepKind) -> f64 {
    match kind {
        StepKind::EncodeVideo => 40.0,
        StepKind::EncodeAudio | StepKind::MeasureLoudness => 2.0,
        StepKind::ExtractMetadata
        | StepKind::ExtractSubtitle
        | StepKind::FilterSubtitles
        | StepKind::OcrSubtitles
        | StepKind::AdjustActiveArea
        | StepKind::ExtractTimestamps
        | StepKind::Mux
        | StepKind::ChooseQuality
        | StepKind::Verify => 1.0,
        StepKind::WriteFile => 0.1,
    }
}

impl QueueRunner {
    /// Start the scheduler. The queue begins *held*: nothing runs until
    /// [`QueueRunner::start`], so jobs left from a previous session never
    /// start by surprise.
    pub fn new(store: QueueStore, on_event: impl Fn(QueueEvent) + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Shared {
            store: Mutex::new(store),
            state: Mutex::new(State {
                held: true,
                shutdown: false,
                running: HashMap::new(),
            }),
            wake: Condvar::new(),
            on_event: Box::new(on_event),
        });
        let s = Arc::clone(&shared);
        std::thread::spawn(move || scheduler(s));
        QueueRunner { shared }
    }

    fn store(&self) -> std::sync::MutexGuard<'_, QueueStore> {
        self.shared.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn changed(&self) {
        (self.shared.on_event)(QueueEvent::Changed);
        self.shared.wake.notify_all();
    }

    pub fn add(&self, job: &JobFile) -> Result<i64> {
        let id = self.store().add(job)?;
        self.changed();
        Ok(id)
    }

    pub fn list(&self) -> Result<Vec<QueueRow>> {
        let mut rows = self.store().list()?;
        // Reflect live pause state for running jobs.
        let state = self.state();
        for r in rows.iter_mut() {
            if let Some(run) = state.running.get(&r.id) {
                r.status = if run.pause.is_paused() {
                    JobStatus::Paused
                } else {
                    JobStatus::Running
                };
            }
        }
        Ok(rows)
    }

    /// The stored job file (plan, preset, tool versions) for a queued job.
    pub fn job(&self, id: i64) -> Result<JobFile> {
        self.store().job(id)
    }

    pub fn progress(&self, id: i64) -> Option<LiveProgress> {
        self.state().running.get(&id).map(|r| r.progress.clone())
    }

    pub fn set_thumbnail(&self, id: i64, path: &str) -> Result<()> {
        self.store().set_thumbnail(id, path)?;
        self.changed();
        Ok(())
    }

    /// Let the queue run pending jobs.
    pub fn start(&self) {
        self.state().held = false;
        self.changed();
    }

    /// Stop starting new jobs; running jobs carry on.
    pub fn hold(&self) {
        self.state().held = true;
        self.changed();
    }

    pub fn is_held(&self) -> bool {
        self.state().held
    }

    pub fn running_count(&self) -> usize {
        self.state().running.len()
    }

    pub fn pause(&self, id: i64) {
        if let Some(r) = self.state().running.get(&id) {
            r.pause.pause();
        }
        self.changed();
    }

    pub fn resume(&self, id: i64) {
        if let Some(r) = self.state().running.get(&id) {
            r.pause.resume();
        }
        self.changed();
    }

    /// Cancel a running job, or take a pending one out of the run order.
    pub fn cancel(&self, id: i64) -> Result<()> {
        let running = self.state().running.get(&id).map(|r| {
            r.pause.resume();
            r.cancel.cancel();
        });
        if running.is_none() && self.store().status(id)? == Some(JobStatus::Pending) {
            self.store()
                .set_status(id, JobStatus::Cancelled, "Cancelled before it started.")?;
        }
        self.changed();
        Ok(())
    }

    pub fn retry(&self, id: i64) -> Result<()> {
        if !self.state().running.contains_key(&id) {
            self.store().retry(id)?;
        }
        self.changed();
        Ok(())
    }

    pub fn remove(&self, id: i64) -> Result<()> {
        if self.state().running.contains_key(&id) {
            return Err(Error::Invalid {
                field: "queue",
                reason: "cancel the job before removing it".into(),
            });
        }
        self.store().remove(id)?;
        self.changed();
        Ok(())
    }

    pub fn move_by(&self, id: i64, delta: i64) -> Result<()> {
        self.store().move_by(id, delta)?;
        self.changed();
        Ok(())
    }

    pub fn clear_finished(&self) -> Result<usize> {
        let n = self.store().clear_finished()?;
        self.changed();
        Ok(n)
    }

    /// Cancel everything and stop the scheduler (app closing). Cancelled
    /// running jobs are recorded as interrupted so they can be retried.
    pub fn shutdown(&self) {
        let mut state = self.state();
        state.shutdown = true;
        for r in state.running.values() {
            r.pause.resume();
            r.cancel.cancel();
        }
        drop(state);
        self.shared.wake.notify_all();
        // Give jobs a moment to stop and record their state.
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.running_count() > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

fn scheduler(shared: Arc<Shared>) {
    loop {
        let next = {
            let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.shutdown {
                return;
            }
            if state.held {
                None
            } else {
                let busy: Vec<Slot> = state.running.values().map(|r| r.slot.clone()).collect();
                drop(state);
                let store = shared.store.lock().unwrap_or_else(|e| e.into_inner());
                store.next_runnable(&busy).ok().flatten()
            }
        };

        if let Some((id, job)) = next {
            launch(&shared, id, job);
            continue;
        }
        let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let _ = shared.wake.wait_timeout(state, Duration::from_millis(500));
    }
}

fn launch(shared: &Arc<Shared>, id: i64, job: JobFile) {
    let plan = job.plan;
    let cancel = CancelToken::new();
    let pause = PauseToken::new();
    let total_weight: f64 = plan
        .steps
        .iter()
        .map(|s| weight(&s.kind))
        .sum::<f64>()
        .max(1e-9);
    {
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.running.insert(
            id,
            Running {
                slot: slot_for(&plan),
                cancel: cancel.clone(),
                pause: pause.clone(),
                progress: LiveProgress {
                    steps_total: plan.steps.len(),
                    step: "Starting".into(),
                    ..Default::default()
                },
            },
        );
    }
    {
        let store = shared.store.lock().unwrap_or_else(|e| e.into_inner());
        let _ = store.set_status(id, JobStatus::Running, "");
    }
    (shared.on_event)(QueueEvent::Changed);

    let shared = Arc::clone(shared);
    std::thread::spawn(move || {
        let started = Instant::now();
        let mut done_weight = 0.0;
        let mut finished_steps = 0usize;
        let mut running: BTreeMap<usize, f64> = BTreeMap::new();
        let mut detail: Option<(usize, Progress)> = None;
        let mut last_emit = Instant::now() - Duration::from_secs(1);

        let result = run_plan_with(&plan, &cancel, &pause, |event| {
            match &event {
                Event::StepStarted { index, .. } => {
                    running.insert(*index, 0.0);
                }
                Event::StepProgress { index, progress } => {
                    running.insert(*index, progress.fraction.unwrap_or(0.0));
                    // Prefer the video encode's numbers for the detail line.
                    let is_video = plan.steps[*index].kind == StepKind::EncodeVideo;
                    if is_video || detail.as_ref().is_none_or(|(i, _)| *i == *index) {
                        detail = Some((*index, progress.clone()));
                    }
                }
                Event::StepFinished { index, .. } => {
                    running.remove(index);
                    finished_steps += 1;
                    done_weight += weight(&plan.steps[*index].kind);
                    if detail.as_ref().is_some_and(|(i, _)| i == index) {
                        detail = None;
                    }
                }
                Event::Verified { report } => {
                    let store = shared.store.lock().unwrap_or_else(|e| e.into_inner());
                    let _ = store.set_verify(id, report);
                }
            }
            let partial: f64 = running
                .iter()
                .map(|(i, f)| weight(&plan.steps[*i].kind) * f)
                .sum();
            let main = detail
                .as_ref()
                .map(|(i, _)| *i)
                .or_else(|| running.keys().next().copied());
            let live = LiveProgress {
                fraction: ((done_weight + partial) / total_weight).min(1.0),
                step: main
                    .map(|i| plan.steps[i].label.clone())
                    .unwrap_or_else(|| "Finishing".into()),
                steps_done: finished_steps,
                steps_total: plan.steps.len(),
                detail: detail.as_ref().map(|(_, p)| p.clone()),
                elapsed_secs: started.elapsed().as_secs(),
            };
            {
                let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(r) = state.running.get_mut(&id) {
                    r.progress = live;
                }
            }
            let important = !matches!(event, Event::StepProgress { .. });
            if important || last_emit.elapsed() >= Duration::from_millis(250) {
                (shared.on_event)(QueueEvent::Progress(id));
                last_emit = Instant::now();
            }
        });

        let shutting_down = shared.state.lock().map(|s| s.shutdown).unwrap_or(true);
        let (status, message) = match &result {
            Ok(report) => (
                JobStatus::Done,
                format!(
                    "Finished in {}",
                    crate::progress::format_duration(report.secs.round() as u64)
                ),
            ),
            Err(Error::Cancelled) if shutting_down => (
                JobStatus::Interrupted,
                "The app closed while this job was running.".into(),
            ),
            Err(Error::Cancelled) => (JobStatus::Cancelled, "Cancelled.".into()),
            Err(e) => (JobStatus::Failed, e.to_string()),
        };
        if result.is_ok() {
            crate::runner::clean_intermediates(&plan);
        }
        {
            let store = shared.store.lock().unwrap_or_else(|e| e.into_inner());
            let _ = store.set_status(id, status, &message);
        }
        {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.running.remove(&id);
        }
        (shared.on_event)(QueueEvent::Changed);
        shared.wake.notify_all();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::pipeline::Step;
    use crate::preset::{PresetStore, DEFAULT_PRESET};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A job whose only step runs a shell snippet, so tests need no media.
    fn job(name: &str, script: &str, encoder: Option<EncoderKind>) -> JobFile {
        let json = include_str!("../tests/fixtures/uhd_dovi_p7_truehd.json");
        let src = crate::probe::parse_ffprobe_json(json).unwrap();
        let preset = PresetStore::builtin()
            .unwrap()
            .resolve(DEFAULT_PRESET, None)
            .unwrap();
        let dir = std::env::temp_dir().join(format!("anvil-queue-{}-{name}", std::process::id()));
        let opts = crate::pipeline::PlanOptions {
            work_dir: dir.clone(),
            output: dir.join("out.mkv"),
            ..Default::default()
        };
        let mut plan = crate::pipeline::plan_job(&src, &preset, &opts).unwrap();
        plan.source = format!("/rips/{name}.mkv");
        plan.encoder = encoder;
        plan.steps = vec![Step {
            label: format!("Work {name}"),
            kind: StepKind::EncodeAudio,
            command: Some(Command::new("sh").args(["-c", script])),
            file: None,
            depends_on: vec![],
            ..Default::default()
        }];
        JobFile::new(DEFAULT_PRESET, preset, plan, Default::default())
    }

    fn wait_until(what: &str, f: impl Fn() -> bool) {
        let start = Instant::now();
        while !f() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn store_persists_orders_and_recovers_interrupted_jobs() {
        let path = std::env::temp_dir().join(format!("anvil-queue-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let (a, b);
        {
            let mut s = QueueStore::open(&path).unwrap();
            a = s.add(&job("a", "true", None)).unwrap();
            b = s.add(&job("b", "true", None)).unwrap();
            s.set_status(a, JobStatus::Running, "").unwrap();
            s.move_by(b, -1).unwrap();
            assert_eq!(
                s.list().unwrap().iter().map(|r| r.id).collect::<Vec<_>>(),
                vec![b, a]
            );
        }
        // Reopen, as after an app restart.
        let s = QueueStore::open(&path).unwrap();
        let rows = s.list().unwrap();
        assert_eq!(rows[1].status, JobStatus::Interrupted);
        assert_eq!(rows[0].title, "b.mkv");
        assert!(rows[0].badges.contains(&"Dolby Vision 8.1".to_string()));
        s.retry(a).unwrap();
        assert_eq!(s.status(a).unwrap(), Some(JobStatus::Pending));
        assert_eq!(s.job(a).unwrap().plan.steps.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn runner_respects_hold_order_and_slots() {
        let events = Arc::new(AtomicUsize::new(0));
        let e2 = Arc::clone(&events);
        let q = QueueRunner::new(QueueStore::open_in_memory().unwrap(), move |_| {
            e2.fetch_add(1, Ordering::SeqCst);
        });
        let marker = std::env::temp_dir().join(format!("anvil-queue-order-{}", std::process::id()));
        let m = marker.display().to_string();
        let _ = std::fs::remove_file(&marker);
        let cpu1 = q
            .add(&job(
                "cpu1",
                &format!("sleep 0.4; echo cpu1 >> '{m}'"),
                Some(EncoderKind::X265),
            ))
            .unwrap();
        let cpu2 = q
            .add(&job(
                "cpu2",
                &format!("echo cpu2 >> '{m}'"),
                Some(EncoderKind::X265),
            ))
            .unwrap();
        let gpu = q
            .add(&job(
                "gpu",
                &format!("echo gpu >> '{m}'"),
                Some(EncoderKind::NvEncC),
            ))
            .unwrap();

        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(q.running_count(), 0, "held queue runs nothing");
        q.start();
        wait_until("all done", || {
            q.list()
                .unwrap()
                .iter()
                .all(|r| r.status == JobStatus::Done)
        });
        // The GPU job ran alongside the first CPU job; the second CPU job waited.
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap(),
            "gpu\ncpu1\ncpu2\n"
        );
        let rows = q.list().unwrap();
        assert!(rows
            .iter()
            .all(|r| r.started.is_some() && r.finished.is_some()));
        assert!(events.load(Ordering::SeqCst) > 3);
        let _ = (cpu1, cpu2, gpu);
    }

    #[test]
    fn cancel_pause_and_retry() {
        let q = QueueRunner::new(QueueStore::open_in_memory().unwrap(), |_| {});
        let long = q
            .add(&job("long", "sleep 30", Some(EncoderKind::X265)))
            .unwrap();
        let waiting = q
            .add(&job("waiting", "true", Some(EncoderKind::X265)))
            .unwrap();
        q.start();
        wait_until("running", || q.progress(long).is_some());
        q.pause(long);
        assert_eq!(q.list().unwrap()[0].status, JobStatus::Paused);
        q.resume(long);
        q.hold();
        q.cancel(long).unwrap();
        wait_until("cancelled", || {
            q.list().unwrap()[0].status == JobStatus::Cancelled
        });
        assert_eq!(
            q.list().unwrap()[1].status,
            JobStatus::Pending,
            "held queue does not start the next job"
        );
        assert!(q.remove(waiting).is_ok());
        q.retry(long).unwrap();
        assert_eq!(q.list().unwrap()[0].status, JobStatus::Pending);
        q.shutdown();
    }

    #[test]
    fn failures_are_recorded() {
        let q = QueueRunner::new(QueueStore::open_in_memory().unwrap(), |_| {});
        let id = q
            .add(&job(
                "bad",
                "echo 'x265 [error]: bad things' >&2; exit 1",
                Some(EncoderKind::X265),
            ))
            .unwrap();
        q.start();
        wait_until("failed", || {
            q.list().unwrap()[0].status == JobStatus::Failed
        });
        let row = &q.list().unwrap()[0];
        assert!(row.message.contains("bad things"), "{}", row.message);
        let _ = id;
    }
}
