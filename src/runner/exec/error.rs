use anyhow::{Error, Result};
use camino::Utf8Path;
use chrono::Utc;

use crate::{
    state::{RunState, RunStatus, StepStatus},
    store::write_state,
    wake::{self, WakeKind},
};

use super::status::append_run_status;

pub(in crate::runner::exec) fn record_step_error_or_context(
    err: Error,
    state: &mut RunState,
    state_path: &Utf8Path,
    run_dir: &Utf8Path,
    index: usize,
    phase: &str,
) -> Error {
    let record_result = mark_step_failed(state, state_path, index).and_then(|()| {
        append_run_status(
            run_dir,
            &wake::step_line(
                WakeKind::Failed,
                index,
                &format!("{phase} error: {}", status_detail(&err)),
            ),
        )
    });
    match record_result {
        Ok(()) => err,
        Err(record_err) => err.context(format!(
            "additionally failed to record {phase} failure for step {index}: {record_err}"
        )),
    }
}

fn mark_step_failed(state: &mut RunState, state_path: &Utf8Path, index: usize) -> Result<()> {
    let now = Utc::now();
    if let Some(step) = state.steps.iter_mut().find(|step| step.index == index) {
        if step.started_at.is_none() {
            step.started_at = Some(now);
        }
        step.status = StepStatus::Failed;
        step.finished_at = Some(now);
    }
    state.status = RunStatus::Failed;
    state.updated_at = now;
    write_state(state_path, state)
}

fn status_detail(err: &Error) -> String {
    // Status-log wake signals are one line, so collapse multi-line error detail.
    err.to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
