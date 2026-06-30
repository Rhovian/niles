use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    state::{RunState, StepRecord},
    util::write_json_pretty,
};

pub fn write_state(path: &Utf8Path, state: &RunState) -> Result<()> {
    write_json_pretty(path, state)
}

pub fn read_state(run_dir: &Utf8Path) -> Result<RunState> {
    let path = state_path(run_dir);
    let body = fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
    serde_json::from_str(&body).with_context(|| format!("failed to parse {path}"))
}

pub fn state_path(run_dir: &Utf8Path) -> Utf8PathBuf {
    run_dir.join("state.json")
}

pub fn resolve_run_dir(run: &str) -> Result<Utf8PathBuf> {
    let runs_dir = Utf8Path::new(".niles").join("runs");

    if run != "latest" {
        return Ok(runs_dir.join(run));
    }

    let mut runs = fs::read_dir(&runs_dir)
        .with_context(|| format!("failed to read {runs_dir}"))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = Utf8PathBuf::from_path_buf(entry.path()).ok()?;
            path.is_dir().then_some(path)
        })
        .collect::<Vec<_>>();

    runs.sort();
    runs.pop().context("no runs found")
}

pub fn selected_step(state: &RunState, step: Option<usize>) -> Result<&StepRecord> {
    match step {
        Some(step) => state
            .steps
            .iter()
            .find(|record| record.index == step)
            .with_context(|| format!("step {step} not found")),
        None => state.steps.last().context("run has no recorded steps"),
    }
}
