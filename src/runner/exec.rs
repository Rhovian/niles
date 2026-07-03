use std::{fs, io::Write};

use anyhow::{Context, Error, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::{
    agent_window, capabilities,
    config::{
        agents,
        spec::{PromptMode, TaskSpec, TaskStep},
    },
    context::{agent_prompt, write_agent_context},
    process::{ProcessSpec, run_process},
    state::{RunState, RunStatus, StepKind, StepRecord, StepStatus},
    store::{read_state, state_path, write_state},
    util::{absolute_path, append_line, render_template, slugify},
    wake::{self, WakeKind},
};

use super::{RunSelector, lifecycle::load_spec_for_run, report};

/// Scrollback lines captured from an interactive step window on close. Large
/// enough to hold an agent's session; `context.rs` truncates when embedding.
const PANE_CAPTURE_LINES: usize = 2000;
const STEP_WAKE_CONTRACT_TEMPLATE: &str = include_str!("../templates/step_wake_contract.md");

struct LoadedRun {
    run_dir: Utf8PathBuf,
    state_path: Utf8PathBuf,
    state: RunState,
}

fn load_run(selector: RunSelector) -> Result<LoadedRun> {
    let run_dir = selector.resolve()?;
    let state_path = state_path(&run_dir);
    let state = read_state(&run_dir)?;
    Ok(LoadedRun {
        run_dir,
        state_path,
        state,
    })
}

/// Launch a single pending step into its own tmux window. The window runs
/// `niles exec-step`, so output streams live in the pane while state, diff, and
/// exit code are captured exactly as in direct step execution. Completion
/// appends a wake line to the run status log for the manager.
pub(crate) fn step(selector: RunSelector, index: Option<usize>) -> Result<()> {
    let mut run = load_run(selector)?;

    let step_number = match index {
        Some(index) => index,
        None => run
            .state
            .steps
            .iter()
            .find(|step| matches!(step.status, StepStatus::Pending))
            .map(|step| step.index)
            .context("run has no pending steps to launch")?,
    };

    let record = run
        .state
        .steps
        .iter()
        .find(|step| step.index == step_number)
        .with_context(|| format!("step {step_number} not found"))?;
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
    let agent_config = agents::config_for(&spec.agents, agent)?;
    capabilities::validate_agent(
        workspace,
        agent,
        agent_config,
        agents::InvocationDefaults::Default,
    )?;
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
    mark_step_running(&mut run.state, step_number, Some(brief.clone()));
    if let Some(step) = run
        .state
        .steps
        .iter_mut()
        .find(|step| step.index == step_number)
    {
        step.window = Some(window_name.clone());
    }
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
    // Use the window name recorded at launch; fall back to the legacy scheme for
    // runs prepared before window names were stored.
    let window_name = record
        .window
        .clone()
        .unwrap_or_else(|| agent_window::legacy_step_window_name(&run.state.id, index));

    // Capture the interactive pane before tearing it down, so the step's output
    // reaches later steps' handoff context. Best-effort: a window that already
    // exited leaves no pane, and that must not block closing the step.
    let captured = match agent_window::capture_window(&window_name, PANE_CAPTURE_LINES) {
        Ok(text) => {
            let steps_dir = ensure_steps_dir(&run.run_dir)?;
            let path = steps_dir.join(format!("{index:03}-{}.pane.txt", slugify(&label)));
            fs::write(&path, text).with_context(|| format!("failed to write {path}"))?;
            Some(path)
        }
        Err(err) => {
            println!("pane not captured for step {index}: {err}");
            None
        }
    };

    let step = run
        .state
        .steps
        .iter_mut()
        .find(|step| step.index == index)
        .with_context(|| format!("step {index} not found in run"))?;
    step.status = StepStatus::Completed;
    step.finished_at = Some(Utc::now());
    if step.exit_code.is_none() {
        step.exit_code = Some(0);
    }
    if let Some(path) = captured {
        step.stdout = Some(path);
    }

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

    match agent_window::close_window(&window_name) {
        Ok(()) => println!("closed: {window_name}"),
        Err(err) => println!("window {window_name} not closed: {err}"),
    }
    println!("step {index}: completed");
    if all_completed {
        println!("status: completed");
    }
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

fn task_step(spec: &TaskSpec, step_number: usize) -> Result<&TaskStep> {
    let position = step_position(step_number)?;
    spec.steps
        .get(position)
        .with_context(|| format!("step {step_number} is out of range for this task"))
}

fn step_position(step_number: usize) -> Result<usize> {
    step_number
        .checked_sub(1)
        .context("step index must be >= 1")
}

fn spec_workspace(spec: &TaskSpec) -> &Utf8Path {
    spec.workspace.as_deref().unwrap_or(Utf8Path::new("."))
}

fn ensure_steps_dir(run_dir: &Utf8Path) -> Result<Utf8PathBuf> {
    let steps_dir = run_dir.join("steps");
    fs::create_dir_all(&steps_dir).with_context(|| format!("failed to create {steps_dir}"))?;
    Ok(steps_dir)
}

/// Execute one run step in-process (invoked inside the per-step tmux window).
/// Records state for one step, then appends a `done:` or `failed:` wake line to
/// the run status log.
pub(crate) fn exec_step(selector: RunSelector, index: usize) -> Result<()> {
    let mut run = load_run(selector)?;

    let spec = load_spec_for_run(&run.state)?;

    let step = task_step(&spec, index)?;

    let workspace = spec_workspace(&spec);
    if let TaskStep::Agent { agent, .. } = step {
        let agent_config = agents::config_for(&spec.agents, agent)?;
        capabilities::validate_agent(
            workspace,
            agent,
            agent_config,
            agents::InvocationDefaults::Default,
        )?;
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
    let exit = record
        .exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_owned());
    let label = record.label.clone();
    let role_label = record
        .role
        .as_deref()
        .map(|role| format!("{role} "))
        .unwrap_or_default();

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

fn append_run_status(run_dir: &Utf8Path, line: &str) -> Result<()> {
    let path = wake::status_log_path(run_dir);
    append_line(
        &path,
        line,
        |path| format!("failed to open {path}"),
        |path| format!("failed to inspect {path} before status append"),
        |path| format!("failed to write {path}"),
    )
}

fn record_step_error(
    state: &mut RunState,
    state_path: &Utf8Path,
    run_dir: &Utf8Path,
    index: usize,
    phase: &str,
    err: &Error,
) -> Result<()> {
    mark_step_failed(state, state_path, index)?;
    append_run_status(
        run_dir,
        &wake::step_line(
            WakeKind::Failed,
            index,
            &format!("{phase} error: {}", status_detail(err)),
        ),
    )
}

fn record_step_error_or_context(
    err: Error,
    state: &mut RunState,
    state_path: &Utf8Path,
    run_dir: &Utf8Path,
    index: usize,
    phase: &str,
) -> Error {
    match record_step_error(state, state_path, run_dir, index, phase, &err) {
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
    err.to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn ensure_step_brief_exists(brief: &Utf8Path, step_number: usize) -> Result<()> {
    if brief.is_file() {
        Ok(())
    } else {
        bail!("cannot launch step {step_number}: brief does not exist at {brief}")
    }
}

/// Run one step: build its handoff context, mark it running, execute it, and
/// record the result. Returns whether the step failed. Run-level status is left
/// at `Running`/`Failed` for the caller to finalize.
#[allow(clippy::too_many_arguments)]
pub(in crate::runner) fn execute_single_step(
    spec: &TaskSpec,
    state: &mut RunState,
    state_path: &Utf8Path,
    steps_dir: &Utf8Path,
    workspace: &Utf8Path,
    step: &TaskStep,
    step_number: usize,
    watch: bool,
) -> Result<bool> {
    let context = match step {
        TaskStep::Agent { agent, task, role } => Some(write_agent_context(
            state,
            step_number,
            role.as_deref(),
            agent,
            task,
            workspace,
            steps_dir,
        )?),
        TaskStep::Command { .. } => None,
        TaskStep::Role { role, .. } => {
            bail!("step {step_number} role `{role}` was not resolved; check .niles/manifest.yaml")
        }
    };

    mark_step_running(state, step_number, context.clone());
    state.updated_at = Utc::now();
    write_state(state_path, state)?;
    if watch {
        report::print_watch_snapshot(state);
    }

    let result = match step {
        TaskStep::Agent { agent, task, role } => {
            print_step_start(step_number, role.as_deref(), "agent", agent);
            print_step_context(context.as_deref());
            run_agent_step(AgentStep {
                step_number,
                role: role.clone(),
                agent,
                task,
                spec,
                workspace,
                steps_dir,
                context_path: context,
            })
        }
        TaskStep::Command { command, role } => {
            print_step_start(step_number, role.as_deref(), "command", command);
            run_command_step(
                step_number,
                role.clone(),
                command,
                spec,
                workspace,
                steps_dir,
            )
        }
        TaskStep::Role { role, .. } => {
            bail!("step {step_number} role `{role}` was not resolved; check .niles/manifest.yaml")
        }
    }?;

    let failed = matches!(result.status, StepStatus::Failed);
    update_step_record(state, result);
    state.status = if failed {
        RunStatus::Failed
    } else {
        RunStatus::Running
    };
    state.updated_at = Utc::now();
    write_state(state_path, state)?;
    if watch {
        report::print_watch_snapshot(state);
    }

    Ok(failed)
}

fn mark_step_running(state: &mut RunState, step_number: usize, context: Option<Utf8PathBuf>) {
    if let Some(step) = state
        .steps
        .iter_mut()
        .find(|step| step.index == step_number)
    {
        step.status = StepStatus::Running;
        step.started_at = Some(Utc::now());
        step.context = context;
    }
}

fn update_step_record(state: &mut RunState, result: StepRecord) {
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

struct AgentStep<'a> {
    step_number: usize,
    role: Option<String>,
    agent: &'a str,
    task: &'a str,
    spec: &'a TaskSpec,
    workspace: &'a Utf8Path,
    steps_dir: &'a Utf8Path,
    context_path: Option<Utf8PathBuf>,
}

fn run_agent_step(step: AgentStep<'_>) -> Result<StepRecord> {
    let config = agents::config_for(&step.spec.agents, step.agent)?;
    let config = agents::invocation(step.agent, config, agents::InvocationDefaults::Default)?;
    let mut args = config.args;
    let prompt = agent_prompt(step.task, step.context_path.as_deref())?;
    let stdin = match config.prompt {
        PromptMode::Arg => {
            args.push(prompt);
            None
        }
        PromptMode::Stdin => Some(prompt),
    };

    let mut record = run_process(ProcessSpec {
        step_number: step.step_number,
        role: step.role,
        kind: StepKind::Agent,
        label: step.agent,
        binary: &config.binary,
        args: &args,
        stdin: stdin.as_deref(),
        workspace: step.workspace,
        steps_dir: step.steps_dir,
        context_path: step.context_path,
    })?;
    apply_agent_tier(&mut record, &config.spec);
    Ok(record)
}

fn apply_agent_tier(record: &mut StepRecord, spec: &agents::AgentSpec) {
    if let Some(tier) = spec.tier() {
        record.agent_family = Some(tier.family);
        record.model = tier.model;
        record.effort = tier.effort;
    }
}

fn run_command_step(
    step_number: usize,
    role: Option<String>,
    command: &str,
    spec: &TaskSpec,
    workspace: &Utf8Path,
    steps_dir: &Utf8Path,
) -> Result<StepRecord> {
    let config = spec
        .commands
        .get(command)
        .with_context(|| format!("unknown command `{command}`"))?;
    let command_line = config.run();
    let args = ["-c".to_owned(), command_line.to_owned()];
    run_process(ProcessSpec {
        step_number,
        role,
        kind: StepKind::Command,
        label: command,
        binary: "sh",
        args: &args,
        stdin: None,
        workspace,
        steps_dir,
        context_path: None,
    })
}

fn print_step_start(step_number: usize, role: Option<&str>, kind: &str, label: &str) {
    match role {
        Some(role) => println!("step {step_number}: {role} {kind} {label}"),
        None => println!("step {step_number}: {kind} {label}"),
    }
}

fn print_step_context(context: Option<&Utf8Path>) {
    if let Some(context) = context {
        println!("context: {context}");
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
