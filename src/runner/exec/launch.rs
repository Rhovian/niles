use std::{fs, io::Write};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;
#[cfg(test)]
use camino::Utf8PathBuf;
use chrono::Utc;

use crate::{
    agent_window, agents,
    config::spec::TaskStep,
    context::write_agent_context,
    state::{RunStatus, StepStatus},
    store::write_state,
    util::{absolute_path, render_template},
    wake,
};

use super::super::{RunSelector, lifecycle::load_spec_for_run};
use super::{
    error::record_step_error_or_context,
    run::{
        ensure_steps_dir, load_run, mark_step_running, spec_workspace, step_record_position,
        task_step,
    },
};

const STEP_WAKE_CONTRACT_TEMPLATE: &str = include_str!("../../templates/step_wake_contract.md");

/// Launch a single pending step into its own tmux window. The window runs
/// `niles exec-step`, so output streams live in the pane while state, diff, and
/// exit code are captured exactly as in direct step execution. Completion
/// appends a wake line to the run status log for the manager.
pub(crate) fn step(selector: RunSelector, index: Option<usize>) -> Result<()> {
    let mut run = load_run(selector)?;

    let record_position = match index {
        Some(index) => step_record_position(&run.state, index)?,
        None => run
            .state
            .steps
            .iter()
            .position(|step| matches!(step.status, StepStatus::Pending))
            .context("run has no pending steps to launch")?,
    };

    let step_number = run.state.steps[record_position].index;
    let record = &run.state.steps[record_position];
    if !matches!(record.status, StepStatus::Pending) {
        bail!(
            "step {step_number} is {}; only pending steps can be launched",
            record.status
        );
    }

    // Handoff context for step N folds in every prior step's output, so a step
    // can only launch once all earlier steps have completed.
    if let Some(prior) = run
        .state
        .steps
        .iter()
        .find(|step| step.index < step_number && !matches!(step.status, StepStatus::Completed))
    {
        bail!(
            "cannot launch step {step_number}: step {} is {}; prior steps must complete first",
            prior.index,
            prior.status
        );
    }

    let spec = load_spec_for_run(&run.state)?;
    let task_step = task_step(&spec, step_number)?;
    let (agent, task, role) = match task_step {
        TaskStep::Agent { agent, task, role } => (agent, task, role),
        TaskStep::Command { command, .. } => bail!(
            "step {step_number} is the `{command}` command; run it captured with `niles exec-step {} {step_number}`",
            run.state.id
        ),
        TaskStep::Role { role, .. } => {
            bail!("step {step_number} role `{role}` was not resolved; check .niles/manifest.yaml")
        }
    };

    let workspace = spec_workspace(&spec);
    agents::capabilities::validate_task_agent(workspace, &spec.agents, agent)?;
    let steps_dir = ensure_steps_dir(&run.run_dir)?;

    // Brief = handoff context plus a wake contract pointing at this run's log.
    let brief = write_agent_context(
        &run.state,
        step_number,
        role.as_deref(),
        agent,
        task,
        workspace,
        &steps_dir,
    )?;
    append_wake_contract(&brief, &run.run_dir, step_number)?;

    let launch_path = steps_dir.join(format!("{step_number:03}-launch.sh"));
    let window_name =
        agent_window::step_window_name(&run.state.id, step_number, role.as_deref(), agent);
    let cwd = absolute_path(workspace)?;
    ensure_step_brief_exists(&brief, step_number)?;
    if let Err(err) =
        agent_window::spawn_agent_window(&window_name, &cwd, agent, workspace, &brief, &launch_path)
    {
        return Err(record_step_error_or_context(
            err,
            &mut run.state,
            &run.state_path,
            &run.run_dir,
            step_number,
            "launch",
        ));
    }

    // Mark the step running now that the window exists, so a follow-up `step`
    // call won't re-pick this step before it is closed.
    let step = &mut run.state.steps[record_position];
    mark_step_running(step, Some(brief.clone()));
    step.window = Some(window_name.clone());
    if matches!(run.state.status, RunStatus::Created) {
        run.state.status = RunStatus::Running;
    }
    run.state.updated_at = Utc::now();
    write_state(&run.state_path, &run.state)?;

    println!("step: {step_number}");
    println!("agent: {agent}");
    println!("window: {window_name}");
    println!("run: {}", run.state.id);
    println!("brief: {brief}");
    println!("status_log: {}", wake::status_log_path(&run.run_dir));
    println!(
        "on_done: niles step-close {} --index {step_number}",
        run.state.id
    );
    Ok(())
}

/// Append a wake contract to a step brief so the interactive agent reports back
/// to the run status log the manager watches.
fn append_wake_contract(brief: &Utf8Path, run_dir: &Utf8Path, step_number: usize) -> Result<()> {
    let status_log = wake::status_log_path(&absolute_path(run_dir)?);
    let step_token = wake::step_token(step_number);
    let wake_examples = wake::step_contract_examples(step_number, &status_log);
    let footer = render_template(
        STEP_WAKE_CONTRACT_TEMPLATE,
        &[
            ("{step_token}", &step_token),
            ("{wake_examples}", &wake_examples),
        ],
    );
    fs::OpenOptions::new()
        .append(true)
        .open(brief)
        .with_context(|| format!("failed to open {brief}"))?
        .write_all(footer.as_bytes())
        .with_context(|| format!("failed to write {brief}"))
}

fn ensure_step_brief_exists(brief: &Utf8Path, step_number: usize) -> Result<()> {
    if brief.is_file() {
        Ok(())
    } else {
        bail!("cannot launch step {step_number}: brief does not exist at {brief}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn wake_contract_requires_step_token_for_all_actionable_lines() {
        let dir = temp_test_dir("wake-contract");
        let run_dir = Utf8PathBuf::from_path_buf(dir.join("run")).unwrap();
        let brief = Utf8PathBuf::from_path_buf(dir.join("brief.md")).unwrap();
        fs::create_dir_all(&run_dir).unwrap();
        fs::write(&brief, "# Brief\n").unwrap();

        append_wake_contract(&brief, &run_dir, 5).unwrap();

        let body = fs::read_to_string(&brief).unwrap();
        assert!(body.contains("done: step 5 <short result>"));
        assert!(body.contains("failed: step 5 <reason>"));
        assert!(body.contains("blocked: step 5 <blocking issues>"));
        assert!(body.contains("needs-decision: step 5 <decision needed>"));
        assert!(body.contains("must include the `step 5` token pair"));

        fs::remove_dir_all(dir).unwrap();
    }

    fn temp_test_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "niles-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
