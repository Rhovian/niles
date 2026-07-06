use std::fs;

use anyhow::{Context, Result};
use chrono::Utc;

use crate::{
    agent_window,
    state::{RunStatus, StepStatus},
    store::write_state,
    usage::{self, UsageAgent, UsageSnapshotInput, UsageSubject},
    util::slugify,
    wake::{self, WakeKind},
};

use super::super::RunSelector;
use super::{
    run::{ensure_steps_dir, load_run},
    status::append_run_status,
};

/// Scrollback lines captured from an interactive step window on close. Large
/// enough to hold an agent's session; `context.rs` truncates when embedding.
const PANE_CAPTURE_LINES: usize = 2000;

/// Mark a step complete and tear down its interactive window. The manager
/// calls this once it judges the step's work finished (typically after the
/// agent's `done:` wake), giving the human final say over window cleanup.
pub(crate) fn step_close(selector: RunSelector, index: usize) -> Result<()> {
    let mut run = load_run(selector)?;

    let record = run
        .state
        .steps
        .iter()
        .find(|step| step.index == index)
        .with_context(|| format!("step {index} not found in run"))?;
    let label = record.label.clone();
    let role = record.role.clone();
    let agent_family = record.agent_family.clone();
    let model = record.model.clone();
    let effort = record.effort.clone();
    let usage_attribution = record.usage_attribution.clone();
    let started_at = record.started_at;
    // Use the window name recorded at launch; fall back to the legacy scheme for
    // runs prepared before window names were stored.
    let window_name = record
        .window
        .clone()
        .unwrap_or_else(|| agent_window::legacy_step_window_name(&run.state.id, index));
    let steps_dir = ensure_steps_dir(&run.run_dir)?;

    // Capture the interactive pane before tearing it down, so the step's output
    // reaches later steps' handoff context. Best-effort: a window that already
    // exited leaves no pane, and that must not block closing the step.
    let captured = match agent_window::capture_window(&window_name, PANE_CAPTURE_LINES) {
        Ok(text) => {
            let path = steps_dir.join(format!("{index:03}-{}.pane.txt", slugify(&label)));
            fs::write(&path, text).with_context(|| format!("failed to write {path}"))?;
            Some(path)
        }
        Err(err) => {
            println!("pane not captured for step {index}: {err}");
            None
        }
    };

    let window_error = agent_window::close_window(&window_name)
        .err()
        .map(|err| err.to_string());
    let finished_at = Utc::now();
    let usage_path = usage::snapshot_usage(UsageSnapshotInput {
        subject: UsageSubject::RunStep {
            run_id: run.state.id.clone(),
            index,
            role: role.clone(),
            label: label.clone(),
        },
        agent: UsageAgent {
            spec: label.clone(),
            family: agent_family,
            model,
            effort,
        },
        attribution: usage_attribution,
        started_at,
        finished_at,
        output_path: usage::step_usage_path(&steps_dir, index, &label),
    });

    let step = run
        .state
        .steps
        .iter_mut()
        .find(|step| step.index == index)
        .with_context(|| format!("step {index} not found in run"))?;
    step.status = StepStatus::Completed;
    step.finished_at = Some(finished_at);
    if step.exit_code.is_none() {
        step.exit_code = Some(0);
    }
    if let Some(path) = captured {
        step.stdout = Some(path);
    }
    step.usage = usage_path;

    let all_completed = run
        .state
        .steps
        .iter()
        .all(|step| matches!(step.status, StepStatus::Completed));
    if all_completed {
        run.state.status = RunStatus::Completed;
    }
    run.state.updated_at = Utc::now();
    write_state(&run.state_path, &run.state)?;
    append_run_status(&run.run_dir, &wake::step_line(WakeKind::Closed, index, ""))?;

    match window_error {
        Some(err) => println!("window {window_name} not closed: {err}"),
        None => println!("closed: {window_name}"),
    }
    println!("step {index}: completed");
    if all_completed {
        println!("status: completed");
    }
    Ok(())
}
