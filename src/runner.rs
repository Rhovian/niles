use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, Write},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::{
    config::{
        agents,
        spec::{
            AgentConfig, PromptMode, TaskSpec, TaskStep, apply_project_config, load_project_config,
            load_task, summarize_spec,
        },
    },
    context::{agent_prompt, write_agent_context},
    crew,
    process::{ProcessSpec, run_process},
    state::{RunState, RunStatus, StepKind, StepRecord, StepStatus},
    store::{read_state, resolve_run_dir, selected_step, state_path, write_state},
    util::{slugify, timestamp_id, write_json_pretty},
};

/// Scrollback lines captured from an interactive step window on close. Large
/// enough to hold an agent's session; `context.rs` truncates when embedding.
const PANE_CAPTURE_LINES: usize = 2000;

pub struct RunSelector(String);

impl RunSelector {
    pub fn new(run: String) -> Self {
        Self(run)
    }

    fn resolve(&self) -> Result<Utf8PathBuf> {
        resolve_run_dir(&self.0)
    }
}

pub fn ask(agent: String, prompt: Vec<String>) -> Result<()> {
    let spec = ask_spec(agent, prompt);

    create_run(with_project_config(spec)?, None, false)
}

fn ask_spec(agent: String, prompt: Vec<String>) -> TaskSpec {
    let prompt = prompt.join(" ");
    TaskSpec {
        goal: prompt.clone(),
        agents: BTreeMap::new(),
        workspace: None,
        steps: vec![TaskStep::Agent {
            agent,
            task: prompt,
            role: None,
        }],
        commands: BTreeMap::new(),
    }
}

pub fn run(task: Utf8PathBuf, watch: bool, prepare: bool) -> Result<()> {
    let spec = with_project_config(load_task(&task)?)?;
    if prepare {
        prepare_run(spec, Some(task))
    } else {
        create_run(spec, Some(task), watch)
    }
}

pub fn run_manifest(task: Utf8PathBuf, watch: bool) -> Result<()> {
    let spec = with_project_config(load_task(&task)?)?;
    create_run(spec, Some(task), watch)
}

/// Launch a single pending step into its own tmux window. The window runs
/// `niles exec-step`, so output streams live in the pane while state, diff, and
/// exit code are captured exactly as in a batch run. Completion appends a wake
/// line to the run status log for the supervisor.
pub fn step(selector: RunSelector, index: Option<usize>) -> Result<()> {
    let run_dir = selector.resolve()?;
    let state_path = state_path(&run_dir);
    let mut state = read_state(&run_dir)?;

    let step_number = match index {
        Some(index) => index,
        None => state
            .steps
            .iter()
            .find(|step| matches!(step.status, StepStatus::Pending))
            .map(|step| step.index)
            .context("run has no pending steps to launch")?,
    };

    let record = state
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
    if let Some(prior) = state
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

    let task_file = state
        .task_file
        .clone()
        .context("run has no task file; only task-backed runs support stepping")?;
    let spec = with_project_config(load_task(&task_file)?)?;
    let position = step_number.checked_sub(1).context("step index must be >= 1")?;
    let task_step = spec
        .steps
        .get(position)
        .with_context(|| format!("step {step_number} is out of range for this task"))?;
    let (agent, task, role) = match task_step {
        TaskStep::Agent { agent, task, role } => (agent, task, role),
        TaskStep::Command { command, .. } => bail!(
            "step {step_number} is the `{command}` command; run it captured with `niles exec-step {} {step_number}`",
            state.id
        ),
    };

    let workspace = spec.workspace.clone();
    let workspace = workspace.as_deref().unwrap_or(Utf8Path::new("."));
    let steps_dir = run_dir.join("steps");
    fs::create_dir_all(&steps_dir).with_context(|| format!("failed to create {steps_dir}"))?;

    // Brief = handoff context plus a wake contract pointing at this run's log.
    let brief = write_agent_context(
        &state,
        step_number,
        role.as_deref(),
        agent,
        task,
        workspace,
        &steps_dir,
    )?;
    append_wake_contract(&brief, &run_dir, step_number)?;

    let launch_path = steps_dir.join(format!("{step_number:03}-launch.sh"));
    let window_name = format!("niles-{}-s{step_number}", state.id);
    let cwd = absolute(workspace)?;
    let target = crew::spawn_agent_window(&window_name, &cwd, agent, workspace, &brief, &launch_path)?;

    // Mark the step running now that the window exists, so a follow-up `step`
    // call won't re-pick this step before it is closed.
    mark_step_running(&mut state, step_number, Some(brief.clone()));
    if matches!(state.status, RunStatus::Created) {
        state.status = RunStatus::Running;
    }
    state.updated_at = Utc::now();
    write_state(&state_path, &state)?;

    println!("step: {step_number}");
    println!("agent: {agent}");
    println!("window: {target}");
    println!("run: {}", state.id);
    println!("brief: {brief}");
    println!("status_log: {}", run_dir.join("status.log"));
    println!("on_done: niles step-close {} --index {step_number}", state.id);
    Ok(())
}

/// Mark a step complete and tear down its interactive window. The supervisor
/// calls this once it judges the step's work finished (typically after the
/// agent's `done:` wake), giving the human final say over window cleanup.
pub fn step_close(selector: RunSelector, index: usize) -> Result<()> {
    let run_dir = selector.resolve()?;
    let state_path = state_path(&run_dir);
    let mut state = read_state(&run_dir)?;
    let window_name = format!("niles-{}-s{index}", state.id);

    let label = state
        .steps
        .iter()
        .find(|step| step.index == index)
        .with_context(|| format!("step {index} not found in run"))?
        .label
        .clone();

    // Capture the interactive pane before tearing it down, so the step's output
    // reaches later steps' handoff context. Best-effort: a window that already
    // exited leaves no pane, and that must not block closing the step.
    let captured = match crew::capture_window(&window_name, PANE_CAPTURE_LINES) {
        Ok(text) => {
            let steps_dir = run_dir.join("steps");
            fs::create_dir_all(&steps_dir)
                .with_context(|| format!("failed to create {steps_dir}"))?;
            let path = steps_dir.join(format!("{index:03}-{}.pane.txt", slugify(&label)));
            fs::write(&path, text).with_context(|| format!("failed to write {path}"))?;
            Some(path)
        }
        Err(err) => {
            println!("pane not captured for step {index}: {err}");
            None
        }
    };

    let step = state
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

    let all_completed = state
        .steps
        .iter()
        .all(|step| matches!(step.status, StepStatus::Completed));
    if all_completed {
        state.status = RunStatus::Completed;
    }
    state.updated_at = Utc::now();
    write_state(&state_path, &state)?;

    match crew::close_window(&window_name) {
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
/// to the run status log the supervisor watches.
fn append_wake_contract(brief: &Utf8Path, run_dir: &Utf8Path, step_number: usize) -> Result<()> {
    let status_log = absolute(run_dir)?.join("status.log");
    let footer = format!(
        "\n## Wake Contract\n\nWhen this step's work is complete, append one line to the run status log so Niles wakes the supervisor:\n\n```sh\necho \"done: step {step_number} <short result>\" >> {status_log}\n```\n\nUse `failed:`, `blocked:`, or `needs-decision:` instead of `done:` when appropriate. Leave this window open; the supervisor reviews your work and closes it.\n"
    );
    fs::OpenOptions::new()
        .append(true)
        .open(brief)
        .with_context(|| format!("failed to open {brief}"))?
        .write_all(footer.as_bytes())
        .with_context(|| format!("failed to write {brief}"))
}

fn absolute(path: &Utf8Path) -> Result<Utf8PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = utf8_path(env::current_dir().context("failed to read current directory")?)?;
    Ok(cwd.join(path))
}

/// Execute one run step in-process (invoked inside the per-step tmux window).
/// Records state via the same path as a batch run, then appends a `done:` or
/// `failed:` wake line to the run status log.
pub fn exec_step(selector: RunSelector, index: usize) -> Result<()> {
    let run_dir = selector.resolve()?;
    let state_path = state_path(&run_dir);
    let mut state = read_state(&run_dir)?;

    let task_file = state
        .task_file
        .clone()
        .context("run has no task file; only task-backed runs support step execution")?;
    let spec = with_project_config(load_task(&task_file)?)?;

    let position = index.checked_sub(1).context("step index must be >= 1")?;
    let step = spec
        .steps
        .get(position)
        .with_context(|| format!("step {index} is out of range for this task"))?;

    let workspace = spec.workspace.clone();
    let workspace = workspace.as_deref().unwrap_or(Utf8Path::new("."));
    let steps_dir = run_dir.join("steps");
    fs::create_dir_all(&steps_dir).with_context(|| format!("failed to create {steps_dir}"))?;

    if matches!(state.status, RunStatus::Created) {
        state.status = RunStatus::Running;
        state.updated_at = Utc::now();
        write_state(&state_path, &state)?;
    }

    let failed = execute_single_step(
        &spec,
        &mut state,
        &state_path,
        &steps_dir,
        workspace,
        step,
        index,
        false,
    )?;

    let record = state
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
        append_run_status(&run_dir, &format!("failed: step {index} {label} exit {exit}"))?;
        println!("status: failed");
        if let Some(step) = state.steps.iter().find(|step| step.index == index) {
            print_failure_summary(step);
        }
        bail!("step {index} failed");
    }

    let all_completed = state
        .steps
        .iter()
        .all(|step| matches!(step.status, StepStatus::Completed));
    if all_completed {
        state.status = RunStatus::Completed;
        state.updated_at = Utc::now();
        write_state(&state_path, &state)?;
    }

    append_run_status(
        &run_dir,
        &format!("done: step {index} {role_label}{label} exit {exit}"),
    )?;
    println!("step {index}: completed");
    if all_completed {
        println!("status: completed");
    }
    Ok(())
}

fn append_run_status(run_dir: &Utf8Path, line: &str) -> Result<()> {
    let path = run_dir.join("status.log");
    let mut body = line.to_owned();
    body.push('\n');
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open {path}"))?
        .write_all(body.as_bytes())
        .with_context(|| format!("failed to write {path}"))
}

fn utf8_path(path: std::path::PathBuf) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(path)
        .map_err(|path| anyhow::anyhow!("path is not UTF-8: {}", path.display()))
}

pub fn resume(selector: RunSelector, watch: bool) -> Result<()> {
    let run_dir = selector.resolve()?;
    let state_path = state_path(&run_dir);
    let mut state = read_state(&run_dir)?;

    if matches!(state.status, RunStatus::Completed) {
        println!("run: {}", state.id);
        println!("status: completed");
        println!("next: nothing to resume");
        return Ok(());
    }

    let task_file = state
        .task_file
        .clone()
        .context("run has no task file; only task-backed runs can be resumed")?;
    let spec = with_project_config(load_task(&task_file)?)?;
    let resume_from = first_incomplete_step(&state).context("run has no incomplete steps")?;
    validate_resume_shape(&state, &spec)?;
    reset_steps_from(&mut state, &spec, resume_from)?;
    state.status = RunStatus::Running;
    state.updated_at = Utc::now();
    write_state(&state_path, &state)?;

    println!("resume: {}", state.id);
    println!("state: {state_path}");
    println!("from_step: {resume_from}");
    println!("watch: niles watch {}", state.id);
    println!("show: niles show {}", state.id);

    execute_run(&spec, &run_dir, state, &state_path, watch, resume_from)?;
    Ok(())
}

pub fn status(selector: RunSelector, json: bool) -> Result<()> {
    let run_dir = selector.resolve()?;
    let path = state_path(&run_dir);
    if json {
        let state = fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
        println!("{state}");
    } else {
        let state = read_state(&run_dir)?;
        print_status(&state);
    }
    Ok(())
}

pub fn watch(selector: RunSelector, interval: f64, no_clear: bool) -> Result<()> {
    if !interval.is_finite() || interval <= 0.0 {
        bail!("watch interval must be a finite positive number");
    }

    let run_dir = selector.resolve()?;
    let interval = Duration::from_secs_f64(interval);
    let mut first = true;

    loop {
        let state = read_state(&run_dir)?;

        if no_clear {
            if !first {
                println!();
            }
        } else {
            print!("\x1B[2J\x1B[H");
        }

        print_status(&state);
        io::stdout().flush().context("failed to flush stdout")?;
        first = false;

        if matches!(state.status, RunStatus::Completed | RunStatus::Failed) {
            break;
        }

        thread::sleep(interval);
    }

    Ok(())
}

pub fn show(selector: RunSelector) -> Result<()> {
    let run_dir = selector.resolve()?;
    let state = read_state(&run_dir)?;

    println!("run: {}", state.id);
    println!("status: {}", state.status);
    println!("goal: {}", state.goal);
    println!("created: {}", state.created_at);
    println!("updated: {}", state.updated_at);

    if state.steps.is_empty() {
        println!("steps: none");
        return Ok(());
    }

    println!("steps:");
    for step in &state.steps {
        println!(
            "  {}. {}{} {} {}{}{}",
            step.index,
            step.role
                .as_deref()
                .map(|role| format!("{role} "))
                .unwrap_or_default(),
            step.kind,
            step.label,
            step.status,
            step.exit_code
                .map(|code| format!(" ({code})"))
                .unwrap_or_default(),
            step.context
                .as_ref()
                .map(|path| format!(" context {path}"))
                .unwrap_or_default()
        );
    }

    Ok(())
}

fn print_status(state: &RunState) {
    println!("run: {}", state.id);
    println!("status: {}", state.status);
    println!("goal: {}", state.goal);
    println!("updated: {}", state.updated_at);

    if state.steps.is_empty() {
        println!("steps[0]:");
        println!("help[2]:");
        println!("  Run `niles show {}`", state.id);
        println!("  Run `niles status {} --json`", state.id);
        return;
    }

    print_steps_table(state);

    let focus_step = state
        .steps
        .iter()
        .find(|step| matches!(step.status, StepStatus::Failed))
        .or_else(|| {
            state
                .steps
                .iter()
                .find(|step| matches!(step.status, StepStatus::Running))
        })
        .or_else(|| {
            state
                .steps
                .iter()
                .rev()
                .find(|step| matches!(step.status, StepStatus::Completed))
        })
        .or_else(|| state.steps.last());

    if let Some(step) = focus_step {
        if matches!(step.status, StepStatus::Failed) {
            println!("help[4]:");
            println!(
                "  Run `niles log {} --step {} --stderr`",
                state.id, step.index
            );
            println!("  Run `niles diff {} --step {}`", state.id, step.index);
            println!("  Run `niles show {}`", state.id);
            println!("  Run `niles status {} --json`", state.id);
        } else {
            println!("help[4]:");
            println!("  Run `niles log {} --step {}`", state.id, step.index);
            println!("  Run `niles diff {} --step {}`", state.id, step.index);
            println!("  Run `niles show {}`", state.id);
            println!("  Run `niles status {} --json`", state.id);
        }
    }
}

fn print_watch_snapshot(state: &RunState) {
    println!("watch:");
    println!("run: {}", state.id);
    println!("status: {}", state.status);
    println!("updated: {}", state.updated_at);

    if state.steps.is_empty() {
        println!("steps[0]:");
    } else {
        print_steps_table(state);
    }
}

fn print_steps_table(state: &RunState) {
    let has_roles = state.steps.iter().any(|step| step.role.is_some());
    if has_roles {
        println!(
            "steps[{}]{{index,role,kind,label,status,exit}}:",
            state.steps.len()
        );
    } else {
        println!(
            "steps[{}]{{index,kind,label,status,exit}}:",
            state.steps.len()
        );
    }
    for step in &state.steps {
        let exit = step
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "-".to_owned());
        if has_roles {
            println!(
                "  {},{},{},{},{},{}",
                step.index,
                step.role.as_deref().unwrap_or("-"),
                step.kind,
                step.label,
                step.status,
                exit
            );
        } else {
            println!(
                "  {},{},{},{},{}",
                step.index, step.kind, step.label, step.status, exit
            );
        }
    }
}

pub fn log(selector: RunSelector, step: Option<usize>, stderr: bool, both: bool) -> Result<()> {
    let run_dir = selector.resolve()?;
    let state = read_state(&run_dir)?;
    let record = selected_step(&state, step)?;

    if both {
        let stdout = record
            .stdout
            .as_ref()
            .with_context(|| format!("step {} has no stdout log yet", record.index))?;
        let stderr = record
            .stderr
            .as_ref()
            .with_context(|| format!("step {} has no stderr log yet", record.index))?;
        print_log_file("stdout", stdout)?;
        print_log_file("stderr", stderr)?;
    } else if stderr {
        let stderr = record
            .stderr
            .as_ref()
            .with_context(|| format!("step {} has no stderr log yet", record.index))?;
        print!(
            "{}",
            fs::read_to_string(stderr)
                .with_context(|| { format!("failed to read stderr log {stderr}") })?
        );
    } else {
        let stdout = record
            .stdout
            .as_ref()
            .with_context(|| format!("step {} has no stdout log yet", record.index))?;
        print!(
            "{}",
            fs::read_to_string(stdout)
                .with_context(|| { format!("failed to read stdout log {stdout}") })?
        );
    }

    Ok(())
}

pub fn diff(selector: RunSelector, step: Option<usize>) -> Result<()> {
    let run_dir = selector.resolve()?;
    let state = read_state(&run_dir)?;
    let record = selected_step(&state, step)?;
    let diff = record
        .diff
        .as_ref()
        .with_context(|| format!("step {} has no captured diff", record.index))?;
    print!(
        "{}",
        fs::read_to_string(diff).with_context(|| format!("failed to read diff {diff}"))?
    );
    Ok(())
}

fn with_project_config(spec: TaskSpec) -> Result<TaskSpec> {
    let config = load_project_config()?;
    Ok(apply_project_config(spec, config))
}

fn init_run(
    spec: &TaskSpec,
    task_file: Option<Utf8PathBuf>,
) -> Result<(Utf8PathBuf, RunState, Utf8PathBuf)> {
    if spec.steps.is_empty() {
        bail!("task spec must contain at least one step");
    }

    let now = Utc::now();
    let id = timestamp_id(&now);
    let run_dir = Utf8Path::new(".niles").join("runs").join(&id);
    fs::create_dir_all(&run_dir).with_context(|| format!("failed to create {run_dir}"))?;
    let plan = summarize_spec(spec);

    let state = RunState {
        id,
        goal: spec.goal.clone(),
        task_file,
        created_at: now,
        updated_at: now,
        status: RunStatus::Created,
        steps: planned_steps(spec),
    };

    let state_path = run_dir.join("state.json");
    write_state(&state_path, &state)?;

    let plan_path = run_dir.join("plan.json");
    write_json_pretty(&plan_path, &plan)?;

    Ok((run_dir, state, state_path))
}

fn create_run(spec: TaskSpec, task_file: Option<Utf8PathBuf>, watch: bool) -> Result<()> {
    let (run_dir, state, state_path) = init_run(&spec, task_file)?;

    println!("run: {}", state.id);
    println!("state: {state_path}");
    println!("watch: niles watch {}", state.id);
    println!("show: niles show {}", state.id);
    execute_run(&spec, &run_dir, state, &state_path, watch, 1)?;

    Ok(())
}

/// Create a run without executing it, so the foreground supervisor can drive the
/// steps one tmux window at a time via `niles step`.
fn prepare_run(spec: TaskSpec, task_file: Option<Utf8PathBuf>) -> Result<()> {
    let (_run_dir, state, state_path) = init_run(&spec, task_file)?;

    println!("run: {}", state.id);
    println!("state: {state_path}");
    println!("status: {}", state.status);
    println!("steps:");
    for step in &state.steps {
        println!(
            "  {} {}{} {}",
            step.index,
            step.role
                .as_deref()
                .map(|role| format!("{role} "))
                .unwrap_or_default(),
            step.kind,
            step.label
        );
    }
    println!("next: niles step {} --index 1", state.id);

    Ok(())
}

fn execute_run(
    spec: &TaskSpec,
    run_dir: &Utf8Path,
    mut state: RunState,
    state_path: &Utf8Path,
    watch: bool,
    start_step: usize,
) -> Result<()> {
    let workspace = spec.workspace.as_deref().unwrap_or(Utf8Path::new("."));
    let steps_dir = run_dir.join("steps");
    fs::create_dir_all(&steps_dir).with_context(|| format!("failed to create {steps_dir}"))?;

    state.status = RunStatus::Running;
    state.updated_at = Utc::now();
    write_state(state_path, &state)?;

    for (index, step) in spec
        .steps
        .iter()
        .enumerate()
        .skip(start_step.saturating_sub(1))
    {
        let step_number = index + 1;
        let failed = execute_single_step(
            spec,
            &mut state,
            state_path,
            &steps_dir,
            workspace,
            step,
            step_number,
            watch,
        )?;

        if failed {
            println!("status: failed");
            if let Some(step) = state.steps.iter().find(|step| step.index == step_number) {
                print_failure_summary(step);
            }
            bail!("step {step_number} failed");
        }
    }

    state.status = RunStatus::Completed;
    state.updated_at = Utc::now();
    write_state(state_path, &state)?;
    if watch {
        print_watch_snapshot(&state);
    }
    println!("status: completed");

    Ok(())
}

/// Run one step: build its handoff context, mark it running, execute it, and
/// record the result. Returns whether the step failed. Run-level status is left
/// at `Running`/`Failed` for the caller to finalize.
#[allow(clippy::too_many_arguments)]
fn execute_single_step(
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
    };

    mark_step_running(state, step_number, context.clone());
    state.updated_at = Utc::now();
    write_state(state_path, state)?;
    if watch {
        print_watch_snapshot(state);
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
            run_command_step(step_number, role.clone(), command, spec, workspace, steps_dir)
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
        print_watch_snapshot(state);
    }

    Ok(failed)
}

fn planned_steps(spec: &TaskSpec) -> Vec<StepRecord> {
    spec.steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let (role, kind, label) = match step {
                TaskStep::Agent { agent, role, .. } => {
                    (role.clone(), StepKind::Agent, agent.clone())
                }
                TaskStep::Command { command, role } => {
                    (role.clone(), StepKind::Command, command.clone())
                }
            };

            StepRecord {
                index: index + 1,
                role,
                kind,
                label,
                status: StepStatus::Pending,
                started_at: None,
                finished_at: None,
                exit_code: None,
                stdout: None,
                stderr: None,
                diff: None,
                context: None,
            }
        })
        .collect()
}

fn first_incomplete_step(state: &RunState) -> Option<usize> {
    state
        .steps
        .iter()
        .find(|step| !matches!(step.status, StepStatus::Completed))
        .map(|step| step.index)
}

fn validate_resume_shape(state: &RunState, spec: &TaskSpec) -> Result<()> {
    let planned = planned_steps(spec);
    if planned.len() != state.steps.len() {
        bail!(
            "cannot resume: task now has {} steps, but run state has {}",
            planned.len(),
            state.steps.len()
        );
    }

    for (expected, actual) in planned.iter().zip(&state.steps) {
        if expected.index != actual.index
            || expected.role != actual.role
            || expected.kind != actual.kind
            || expected.label != actual.label
        {
            bail!(
                "cannot resume: task step {} no longer matches run state",
                expected.index
            );
        }
    }

    Ok(())
}

fn reset_steps_from(state: &mut RunState, spec: &TaskSpec, start_step: usize) -> Result<()> {
    for planned in planned_steps(spec)
        .into_iter()
        .filter(|step| step.index >= start_step)
    {
        let step = state
            .steps
            .iter_mut()
            .find(|step| step.index == planned.index)
            .with_context(|| format!("run state is missing step {}", planned.index))?;
        *step = planned;
    }

    Ok(())
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
    let config = agent_invocation(step.agent, step.spec.agents.get(step.agent));
    let mut args = config.args;
    let prompt = agent_prompt(step.task, step.context_path.as_deref())?;
    let stdin = match config.prompt {
        PromptMode::Arg => {
            args.push(prompt);
            None
        }
        PromptMode::Stdin => Some(prompt),
    };

    run_process(ProcessSpec {
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
    })
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

struct AgentInvocation {
    binary: String,
    args: Vec<String>,
    prompt: PromptMode,
}

fn agent_invocation(agent: &str, config: Option<&AgentConfig>) -> AgentInvocation {
    let default_binary = agents::default_binary(agent);
    let default_args = agents::default_args(agent);
    let default_prompt = agents::default_prompt(agent);

    match config {
        Some(config) => AgentInvocation {
            binary: config.binary.clone().unwrap_or(default_binary),
            args: if config.args.is_empty() {
                default_args
            } else {
                config.args.clone()
            },
            prompt: config.prompt,
        },
        None => AgentInvocation {
            binary: default_binary,
            args: default_args,
            prompt: default_prompt,
        },
    }
}

fn print_log_file(label: &str, path: &Utf8Path) -> Result<()> {
    println!("==> {label}: {path} <==");
    print!(
        "{}",
        fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?
    );
    Ok(())
}

fn print_failure_summary(step: &StepRecord) {
    eprintln!("failure:");
    eprintln!(
        "  step: {} {}{} {}",
        step.index,
        step.role
            .as_deref()
            .map(|role| format!("{role} "))
            .unwrap_or_default(),
        step.kind,
        step.label
    );
    eprintln!(
        "  exit: {}",
        step.exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_owned())
    );
    if let Some(stderr) = &step.stderr {
        eprintln!("  stderr: {stderr}");
    }
    if let Some(diff) = &step.diff {
        eprintln!("  diff: {diff}");
    }
    eprintln!("stderr tail:");

    match &step.stderr {
        Some(stderr) => match stderr_tail(stderr, 12) {
            Ok(lines) if lines.is_empty() => eprintln!("  <empty>"),
            Ok(lines) => {
                for line in lines {
                    eprintln!("  {line}");
                }
            }
            Err(err) => eprintln!("  <failed to read stderr: {err}>"),
        },
        None => eprintln!("  <no stderr log>"),
    }
}

fn stderr_tail(path: &Utf8Path, max_lines: usize) -> Result<Vec<String>> {
    let body = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    let lines = body
        .lines()
        .rev()
        .take(max_lines)
        .map(str::to_owned)
        .collect::<Vec<_>>();

    Ok(lines.into_iter().rev().collect())
}
