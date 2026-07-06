use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::{
    config::spec::{TaskSpec, TaskStep},
    state::{RunState, StepRecord, StepStatus},
    store::{read_state, state_path},
};

use super::super::RunSelector;

const IMPLICIT_TASK_WORKSPACE: &str = ".";

pub(in crate::runner::exec) struct LoadedRun {
    pub(in crate::runner::exec) run_dir: Utf8PathBuf,
    pub(in crate::runner::exec) state_path: Utf8PathBuf,
    pub(in crate::runner::exec) state: RunState,
}

pub(in crate::runner::exec) fn load_run(selector: RunSelector) -> Result<LoadedRun> {
    let run_dir = selector.resolve()?;
    let state_path = state_path(&run_dir);
    let state = read_state(&run_dir)?;
    Ok(LoadedRun {
        run_dir,
        state_path,
        state,
    })
}

pub(in crate::runner::exec) fn task_step(spec: &TaskSpec, step_number: usize) -> Result<&TaskStep> {
    let position = step_number
        .checked_sub(1)
        .context("step index must be >= 1")?;
    spec.steps
        .get(position)
        .with_context(|| format!("step {step_number} is out of range for this task"))
}

pub(in crate::runner::exec) fn spec_workspace(spec: &TaskSpec) -> &Utf8Path {
    match spec.workspace.as_deref() {
        Some(workspace) => workspace,
        None => Utf8Path::new(IMPLICIT_TASK_WORKSPACE),
    }
}

pub(in crate::runner::exec) fn ensure_steps_dir(run_dir: &Utf8Path) -> Result<Utf8PathBuf> {
    let steps_dir = run_dir.join("steps");
    fs::create_dir_all(&steps_dir).with_context(|| format!("failed to create {steps_dir}"))?;
    Ok(steps_dir)
}

pub(in crate::runner::exec) fn step_record_position(
    state: &RunState,
    step_number: usize,
) -> Result<usize> {
    state
        .steps
        .iter()
        .position(|step| step.index == step_number)
        .with_context(|| format!("step {step_number} not found"))
}

pub(in crate::runner::exec) fn mark_step_running(
    step: &mut StepRecord,
    context: Option<Utf8PathBuf>,
) {
    step.status = StepStatus::Running;
    step.started_at = Some(Utc::now());
    step.context = context;
}

pub(in crate::runner::exec) fn update_step_record(state: &mut RunState, result: StepRecord) {
    if let Some(step) = state
        .steps
        .iter_mut()
        .find(|step| step.index == result.index)
    {
        *step = result;
    } else {
        state.steps.push(result);
    }
}
