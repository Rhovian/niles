use anyhow::{Context, Result, bail};
use chrono::Utc;

use crate::{
    agents,
    config::spec::TaskStep,
    process::{exit_code_label, role_prefix},
    state::{RunStatus, StepStatus},
    store::write_state,
    wake::{self, WakeKind},
};

use super::super::{RunSelector, lifecycle::load_spec_for_run, report, spec_workspace};
use super::{
    error::record_step_error_or_context,
    execute::execute_single_step,
    run::{ensure_steps_dir, load_run, task_step},
    status::append_run_status,
};

/// Execute one run step in-process (invoked inside the per-step tmux window).
/// Records state for one step, then appends a `done:` or `failed:` wake line to
/// the run status log.
pub(crate) fn exec_step(selector: RunSelector, index: usize) -> Result<()> {
    let mut run = load_run(selector)?;

    let spec = load_spec_for_run(&run.state)?;

    let step = task_step(&spec, index)?;

    let workspace = spec_workspace(&spec);
    if let TaskStep::Agent { agent, .. } = step {
        agents::capabilities::validate_task_agent(workspace, &spec.agents, agent)?;
    }
    let steps_dir = ensure_steps_dir(&run.run_dir)?;

    if matches!(run.state.status, RunStatus::Created) {
        run.state.status = RunStatus::Running;
        run.state.updated_at = Utc::now();
        write_state(&run.state_path, &run.state)?;
    }

    let failed = match execute_single_step(
        &spec,
        &mut run.state,
        &run.state_path,
        &steps_dir,
        workspace,
        step,
        index,
        false,
    ) {
        Ok(failed) => failed,
        Err(err) => {
            return Err(record_step_error_or_context(
                err,
                &mut run.state,
                &run.state_path,
                &run.run_dir,
                index,
                "exec",
            ));
        }
    };

    let record = run
        .state
        .steps
        .iter()
        .find(|step| step.index == index)
        .with_context(|| format!("run state is missing step {index}"))?;
    let exit = exit_code_label(record.exit_code);
    let label = record.label.clone();
    let role_label = role_prefix(record.role.as_deref());

    if failed {
        append_run_status(
            &run.run_dir,
            &wake::step_line(WakeKind::Failed, index, &format!("{label} exit {exit}")),
        )?;
        println!("status: failed");
        if let Some(step) = run.state.steps.iter().find(|step| step.index == index) {
            report::print_failure_summary(step);
        }
        bail!("step {index} failed");
    }

    let all_completed = run
        .state
        .steps
        .iter()
        .all(|step| matches!(step.status, StepStatus::Completed));
    if all_completed {
        run.state.status = RunStatus::Completed;
        run.state.updated_at = Utc::now();
        write_state(&run.state_path, &run.state)?;
    }

    append_run_status(
        &run.run_dir,
        &wake::step_line(
            WakeKind::Done,
            index,
            &format!("{role_label}{label} exit {exit}"),
        ),
    )?;
    println!("step {index}: completed");
    if all_completed {
        println!("status: completed");
    }
    Ok(())
}
