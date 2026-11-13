// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Running external tools with a timeout, so a hung or very slow tool can
//! never freeze the app.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct Captured {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Run `program args`, capturing output, killing it after `timeout`.
pub fn run_with_timeout(program: &str, args: &[String], timeout: Duration) -> Result<Captured> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::ToolMissing(program.to_string()),
            _ => Error::Tool {
                tool: program.to_string(),
                message: e.to_string(),
            },
        })?;

    // Drain pipes on threads so a chatty tool cannot block on a full pipe.
    let mut out = child.stdout.take().expect("piped stdout");
    let mut err = child.stderr.take().expect("piped stderr");
    let out_t = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = out.read_to_end(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = err.read_to_end(&mut s);
        s
    });

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Tool {
                    tool: program.to_string(),
                    message: format!("timed out after {}s", timeout.as_secs()),
                });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => {
                return Err(Error::Tool {
                    tool: program.to_string(),
                    message: e.to_string(),
                })
            }
        }
    };

    Ok(Captured {
        success: status.success(),
        stdout: String::from_utf8_lossy(&out_t.join().unwrap_or_default()).into_owned(),
        stderr: String::from_utf8_lossy(&err_t.join().unwrap_or_default()).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_output_and_reports_missing_tools() {
        let ok = run_with_timeout(
            "sh",
            &["-c".into(), "echo hi; echo oops >&2".into()],
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(ok.success);
        assert_eq!(ok.stdout.trim(), "hi");
        assert_eq!(ok.stderr.trim(), "oops");
        assert!(matches!(
            run_with_timeout("definitely-not-a-real-tool", &[], Duration::from_secs(1)),
            Err(Error::ToolMissing(_))
        ));
    }

    #[test]
    fn kills_slow_tools() {
        let start = Instant::now();
        let r = run_with_timeout("sleep", &["10".into()], Duration::from_millis(300));
        assert!(matches!(r, Err(Error::Tool { .. })));
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}
