use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    schema::{self, ArtifactKind},
    state::{RunState, StepRecord},
    util::write_json_pretty,
};

pub fn write_state(path: &Utf8Path, state: &RunState) -> Result<()> {
    write_json_pretty(path, state)
}

pub fn read_state(run_dir: &Utf8Path) -> Result<RunState> {
    let path = state_path(run_dir);
    schema::read_json(&path, ArtifactKind::RunState)
}

pub fn state_path(run_dir: &Utf8Path) -> Utf8PathBuf {
    run_dir.join("state.json")
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
