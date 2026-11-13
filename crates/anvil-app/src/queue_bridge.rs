// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! The QML-facing queue: a thin layer over `anvil_core::queue`.
//!
//! The queue lives in SQLite under `~/.local/share/anvil/`, so it
//! survives restarts. Jobs run on background threads; their events are
//! queued back onto the Qt event loop, where this controller republishes
//! the rows and live progress for QML.
//!
//! The runner is shared process-wide through [`runner()`], so the job
//! editor can add jobs without holding a reference to this object.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// One job per line: id, title, preset, status, badges, output,
        /// thumbnail URL, message, verification summary (tab-separated).
        #[qproperty(QString, rows)]
        /// Running jobs: id, fraction, step, detail, "done/total" (tab-separated).
        #[qproperty(QString, live)]
        /// True when the queue is not starting new jobs.
        #[qproperty(bool, held)]
        #[qproperty(i32, active)]
        #[qproperty(i32, waiting)]
        #[qproperty(QString, notice)]
        type QueueController = super::QueueControllerRust;

        #[qinvokable]
        fn start(self: Pin<&mut QueueController>);
        #[qinvokable]
        fn hold(self: Pin<&mut QueueController>);
        #[qinvokable]
        fn pause(self: Pin<&mut QueueController>, id: i64);
        #[qinvokable]
        fn resume(self: Pin<&mut QueueController>, id: i64);
        #[qinvokable]
        fn cancel(self: Pin<&mut QueueController>, id: i64);
        #[qinvokable]
        fn retry(self: Pin<&mut QueueController>, id: i64);
        #[qinvokable]
        fn remove(self: Pin<&mut QueueController>, id: i64);
        #[qinvokable]
        fn up(self: Pin<&mut QueueController>, id: i64);
        #[qinvokable]
        fn down(self: Pin<&mut QueueController>, id: i64);
        #[qinvokable]
        fn clear(self: Pin<&mut QueueController>);
        /// Open the output's folder in the file manager.
        #[qinvokable]
        fn reveal(self: Pin<&mut QueueController>, id: i64);
        /// Write the job as a standalone bash script next to its output.
        #[qinvokable]
        fn script(self: Pin<&mut QueueController>, id: i64);
        /// Verification checks (and failure details) as display text.
        #[qinvokable]
        fn checks(self: &QueueController, id: i64) -> QString;
        /// Reload rows and progress.
        #[qinvokable]
        fn refresh(self: Pin<&mut QueueController>);
    }

    impl cxx_qt::Threading for QueueController {}
    impl cxx_qt::Initialize for QueueController {}
}

use core::pin::Pin;
use std::path::PathBuf;
use std::sync::OnceLock;

use anvil_core::queue::{JobStatus, QueueEvent, QueueRunner, QueueStore};
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;

static RUNNER: OnceLock<QueueRunner> = OnceLock::new();

/// The process-wide queue, once the Queue controller has started it.
pub fn runner() -> Option<&'static QueueRunner> {
    RUNNER.get()
}

/// `$XDG_DATA_HOME/anvil` (or `~/.local/share/anvil`).
pub fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
        .join("anvil")
}

#[derive(Default)]
pub struct QueueControllerRust {
    rows: QString,
    live: QString,
    held: bool,
    active: i32,
    waiting: i32,
    notice: QString,
}

impl Drop for QueueControllerRust {
    /// App closing: stop running jobs and record them as interrupted so
    /// they can be retried next time.
    fn drop(&mut self) {
        if let Some(r) = RUNNER.get() {
            r.shutdown();
        }
    }
}

fn q(s: &str) -> QString {
    QString::from(s)
}

fn field(s: &str) -> String {
    s.replace(['\t', '\n'], " ")
}

impl cxx_qt::Initialize for qobject::QueueController {
    fn initialize(mut self: Pin<&mut Self>) {
        let path = std::env::var_os("ANVIL_QUEUE_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir().join("queue.sqlite3"));
        let store = match QueueStore::open(&path) {
            Ok(s) => s,
            Err(e) => {
                self.as_mut()
                    .set_notice(q(&format!("Could not open the queue database: {e}")));
                QueueStore::open_in_memory().expect("in-memory queue")
            }
        };
        let thread = self.qt_thread();
        let runner = QueueRunner::new(store, move |event| {
            let _ = thread.queue(move |mut qobj| match event {
                QueueEvent::Changed => qobj.as_mut().refresh(),
                QueueEvent::Progress(_) => qobj.as_mut().publish_live(),
            });
        });
        let _ = RUNNER.set(runner);
        self.refresh();
    }
}

impl qobject::QueueController {
    fn with<T>(&self, f: impl FnOnce(&QueueRunner) -> T) -> Option<T> {
        RUNNER.get().map(f)
    }

    fn report(mut self: Pin<&mut Self>, result: anvil_core::Result<()>) {
        match result {
            Ok(()) => self.as_mut().set_notice(QString::default()),
            Err(e) => self.as_mut().set_notice(q(&e.to_string())),
        }
    }

    pub fn start(self: Pin<&mut Self>) {
        self.with(|r| r.start());
    }

    pub fn hold(self: Pin<&mut Self>) {
        self.with(|r| r.hold());
    }

    pub fn pause(self: Pin<&mut Self>, id: i64) {
        self.with(|r| r.pause(id));
    }

    pub fn resume(self: Pin<&mut Self>, id: i64) {
        self.with(|r| r.resume(id));
    }

    pub fn cancel(self: Pin<&mut Self>, id: i64) {
        let res = self.with(|r| r.cancel(id)).unwrap_or(Ok(()));
        self.report(res);
    }

    pub fn retry(self: Pin<&mut Self>, id: i64) {
        let res = self.with(|r| r.retry(id)).unwrap_or(Ok(()));
        self.report(res);
    }

    pub fn remove(self: Pin<&mut Self>, id: i64) {
        let res = self.with(|r| r.remove(id)).unwrap_or(Ok(()));
        self.report(res);
    }

    pub fn up(self: Pin<&mut Self>, id: i64) {
        let res = self.with(|r| r.move_by(id, -1)).unwrap_or(Ok(()));
        self.report(res);
    }

    pub fn down(self: Pin<&mut Self>, id: i64) {
        let res = self.with(|r| r.move_by(id, 1)).unwrap_or(Ok(()));
        self.report(res);
    }

    pub fn clear(self: Pin<&mut Self>) {
        let res = self
            .with(|r| r.clear_finished().map(|_| ()))
            .unwrap_or(Ok(()));
        self.report(res);
    }

    pub fn reveal(mut self: Pin<&mut Self>, id: i64) {
        let row = self
            .with(|r| r.list().ok())
            .flatten()
            .and_then(|rows| rows.into_iter().find(|r| r.id == id));
        let Some(row) = row else { return };
        let dir = PathBuf::from(&row.output)
            .parent()
            .map(PathBuf::from)
            .unwrap_or_default();
        if std::process::Command::new("xdg-open")
            .arg(&dir)
            .spawn()
            .is_err()
        {
            self.as_mut()
                .set_notice(q(&format!("Output folder: {}", dir.display())));
        }
    }

    pub fn script(mut self: Pin<&mut Self>, id: i64) {
        let Some(job) = self.with(|r| r.job(id)) else {
            return;
        };
        match job {
            Ok(job) => {
                let path = job.plan.output.with_extension("sh");
                let text = anvil_core::pipeline::to_shell_script(&job.plan);
                let result = std::fs::write(&path, text).and_then(|_| {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                });
                match result {
                    Ok(()) => self
                        .as_mut()
                        .set_notice(q(&format!("Saved {}", path.display()))),
                    Err(e) => self
                        .as_mut()
                        .set_notice(q(&format!("Could not write {}: {e}", path.display()))),
                }
            }
            Err(e) => self.as_mut().set_notice(q(&e.to_string())),
        }
    }

    pub fn checks(&self, id: i64) -> QString {
        let rows = self.with(|r| r.list().ok()).flatten().unwrap_or_default();
        let Some(row) = rows.into_iter().find(|r| r.id == id) else {
            return QString::default();
        };
        let mut out = String::new();
        if !row.message.is_empty() && row.status != JobStatus::Done {
            out.push_str(&row.message);
            out.push_str("\n\n");
        }
        if let Some(report) = &row.verify {
            for c in &report.checks {
                let mark = if c.ok {
                    "✔"
                } else if c.critical {
                    "✘"
                } else {
                    "!"
                };
                out.push_str(&format!("{mark}  {}: {}\n", c.name, c.detail));
            }
        }
        q(out.trim_end())
    }

    pub fn refresh(mut self: Pin<&mut Self>) {
        let Some(runner) = RUNNER.get() else { return };
        let rows = runner.list().unwrap_or_default();
        let text = rows
            .iter()
            .map(|r| {
                let verify = match (&r.verify, r.status) {
                    (Some(v), _) => {
                        let failed = v.checks.iter().filter(|c| !c.ok && c.critical).count();
                        if failed == 0 {
                            format!("{} checks passed", v.checks.len())
                        } else {
                            format!("{failed} of {} checks failed", v.checks.len())
                        }
                    }
                    _ => String::new(),
                };
                format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    r.id,
                    field(&r.title),
                    field(&r.preset_name),
                    r.status.as_str(),
                    field(&r.badges.join(" · ")),
                    field(&r.output),
                    r.thumbnail
                        .as_ref()
                        .map(|t| format!("file://{t}"))
                        .unwrap_or_default(),
                    field(&r.message),
                    field(&verify),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let active = rows.iter().filter(|r| r.status.is_active()).count() as i32;
        let waiting = rows
            .iter()
            .filter(|r| r.status == JobStatus::Pending)
            .count() as i32;
        self.as_mut().set_rows(q(&text));
        self.as_mut().set_active(active);
        self.as_mut().set_waiting(waiting);
        self.as_mut().set_held(runner.is_held());
        self.publish_live();
    }

    fn publish_live(mut self: Pin<&mut Self>) {
        let Some(runner) = RUNNER.get() else { return };
        let rows = runner.list().unwrap_or_default();
        let text = rows
            .iter()
            .filter(|r| r.status.is_active())
            .filter_map(|r| runner.progress(r.id).map(|p| (r.id, p)))
            .map(|(id, p)| {
                // Whole-job percent is shown separately, so leave the step's out.
                let detail = p
                    .detail
                    .as_ref()
                    .map(|d| {
                        anvil_core::progress::Progress {
                            fraction: None,
                            ..d.clone()
                        }
                        .summary()
                    })
                    .unwrap_or_default();
                format!(
                    "{id}\t{:.4}\t{}\t{}\t{}/{}",
                    p.fraction,
                    field(&p.step),
                    field(&detail),
                    p.steps_done,
                    p.steps_total
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        if self.rust().live.to_string() != text {
            self.as_mut().set_live(q(&text));
        }
    }
}
