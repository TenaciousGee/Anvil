// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! A process invocation the runner will execute, optionally fed by another
//! process through a pipe (e.g. `vspipe ... | x265 ...`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    /// When set, this command's stdout is piped into ours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin_from: Option<Box<Command>>,
}

impl Command {
    pub fn new(program: impl Into<String>) -> Self {
        Command {
            program: program.into(),
            args: Vec::new(),
            stdin_from: None,
        }
    }

    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }

    pub fn args<I, S>(mut self, it: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(it.into_iter().map(Into::into));
        self
    }

    pub fn piped_from(mut self, upstream: Command) -> Self {
        self.stdin_from = Some(Box::new(upstream));
        self
    }

    /// Render as a copy-pasteable POSIX shell line, for logs and the UI's
    /// command preview.
    pub fn to_shell(&self) -> String {
        let own = std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" ");
        match &self.stdin_from {
            Some(up) => format!("{} | {}", up.to_shell(), own),
            None => own,
        }
    }
}

/// Quote a word for POSIX shells only when needed.
pub fn shell_quote(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=,+@%".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_rendering_and_quoting() {
        let c = Command::new("x265")
            .args(["--input", "-", "--output", "My Film.hevc"])
            .piped_from(Command::new("vspipe").args(["-c", "y4m", "job.vpy", "-"]));
        assert_eq!(
            c.to_shell(),
            "vspipe -c y4m job.vpy - | x265 --input - --output 'My Film.hevc'"
        );
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote("G(1,2)"), "'G(1,2)'");
    }
}
