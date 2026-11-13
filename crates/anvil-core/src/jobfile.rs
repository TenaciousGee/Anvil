// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
//! Job files: a complete, reproducible record of one encode.
//!
//! Saved as `job.toml` in the job's work folder before it runs. It holds the
//! resolved preset (every value, not just the changed ones), the full plan
//! with every command and the generated VapourSynth script, and the version
//! of each tool, so a job can be re-run or audited later with
//! `anvil run --job job.toml`.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::pipeline::JobPlan;
use crate::preset::Preset;
use crate::{Error, Result};

pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobFile {
    pub format: u32,
    /// Job files written before the rename say `reelforge_version`.
    #[serde(alias = "reelforge_version")]
    pub anvil_version: String,
    /// Seconds since the Unix epoch.
    pub created: u64,
    pub preset_id: String,
    /// Tool name -> version string, as detected when the job was created.
    #[serde(default)]
    pub tools: BTreeMap<String, String>,
    pub preset: Preset,
    pub plan: JobPlan,
}

impl JobFile {
    pub fn new(
        preset_id: &str,
        preset: Preset,
        plan: JobPlan,
        tools: BTreeMap<String, String>,
    ) -> Self {
        JobFile {
            format: FORMAT_VERSION,
            anvil_version: env!("CARGO_PKG_VERSION").to_string(),
            created: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            preset_id: preset_id.to_string(),
            tools,
            preset,
            plan,
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self)
            .map_err(|e| Error::Preset(format!("could not write job file: {e}")))?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|source| Error::Io {
                path: dir.into(),
                source,
            })?;
        }
        let header = "# Anvil job file. Re-run with: anvil run --job <this file>\n\n";
        std::fs::write(path, format!("{header}{text}")).map_err(|source| Error::Io {
            path: path.into(),
            source,
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.into(),
            source,
        })?;
        let job: JobFile = toml::from_str(&text)?;
        if job.format > FORMAT_VERSION {
            return Err(Error::Preset(format!(
                "job file format {} is newer than this Anvil understands ({FORMAT_VERSION})",
                job.format
            )));
        }
        Ok(job)
    }
}

/// Tool versions for the job file, from the same detection as the Tools page.
pub fn tool_versions(timeout: std::time::Duration) -> BTreeMap<String, String> {
    crate::tools::detect_all(timeout)
        .into_iter()
        .filter_map(|t| t.version.map(|v| (t.name.to_string(), v)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crop::Crop;
    use crate::pipeline::{plan_job, PlanOptions};
    use crate::preset::{PresetStore, DEFAULT_PRESET};

    #[test]
    fn job_file_round_trip_with_encoded_audio() {
        let json = include_str!("../tests/fixtures/uhd_dovi_p7_truehd.json");
        let mut src = crate::probe::parse_ffprobe_json(json).unwrap();
        // Force an encoded AC-3 track so the tagged audio enum is exercised.
        src.audio
            .retain(|a| a.codec != crate::model::AudioCodec::Ac3);
        let preset = PresetStore::builtin()
            .unwrap()
            .resolve(DEFAULT_PRESET, None)
            .unwrap();
        let opts = PlanOptions {
            work_dir: "/w".into(),
            output: "/o/film.mkv".into(),
            crop: Some(Crop {
                top: 276,
                bottom: 276,
                ..Default::default()
            }),
            ..Default::default()
        };
        let plan = plan_job(&src, &preset, &opts).unwrap();
        let tools = BTreeMap::from([("x265".to_string(), "HEVC encoder version 3.6".to_string())]);
        let job = JobFile::new(DEFAULT_PRESET, preset, plan, tools);

        let path = std::env::temp_dir().join(format!("anvil-job-{}.toml", std::process::id()));
        job.save(&path).unwrap();
        let back = JobFile::load(&path).unwrap();
        assert_eq!(back.preset, job.preset);
        assert_eq!(back.plan.audio, job.plan.audio);
        assert_eq!(back.plan.expect, job.plan.expect);
        assert_eq!(back.plan.steps.len(), job.plan.steps.len());
        let script = back
            .plan
            .steps
            .iter()
            .filter_map(|s| s.file.as_ref())
            .find(|f| f.0.ends_with("job.vpy"))
            .unwrap();
        assert!(script.1.contains("core.std.Crop"));
        assert_eq!(back.tools["x265"], "HEVC encoder version 3.6");
    }
}
