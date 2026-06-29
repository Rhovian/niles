use std::{collections::BTreeMap, fs};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::{
    process::run_process,
    spec::{
        AgentConfig, PromptMode, TaskSpec, TaskStep, apply_project_config, command_config_run,
        load_project_config, load_task, summarize_spec,
    },
    state::{
        RunState, RunStatus, StepKind, StepRecord, StepStatus, run_status_label, step_kind_label,
        step_status_label,
    },
    store::{read_state, resolve_run_dir, selected_step, state_path, write_state},
};

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
    let prompt = prompt.join(" ");
    let spec = TaskSpec {
        goal: prompt.clone(),
        agents: BTreeMap::new(),
        workspace: None,
        steps: vec![TaskStep::Agent {
            agent,
            task: prompt,
            role: None,
        }],
        commands: BTreeMap::new(),
    };

    create_run(with_project_config(spec)?, None)
}

pub fn run(task: Utf8PathBuf) -> Result<()> {
    let spec = load_task(&task)?;
    create_run(with_project_config(spec)?, Some(task))
}

pub fn resume(selector: RunSelector) -> Result<()> {
    let run_dir = selector.resolve()?;
    println!("resume target: {run_dir}");
    println!("next: resume execution is not implemented yet");
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

pub fn show(selector: RunSelector) -> Result<()> {
    let run_dir = selector.resolve()?;
    let state = read_state(&run_dir)?;

    println!("run: {}", state.id);
    println!("status: {}", run_status_label(&state.status));
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
            "  {}. {}{} {} {}{}",
            step.index,
            step.role
                .as_deref()
                .map(|role| format!("{role} "))
                .unwrap_or_default(),
            step_kind_label(&step.kind),
            step.label,
            step_status_label(&step.status),
            step.exit_code
                .map(|code| format!(" ({code})"))
                .unwrap_or_default()
        );
    }

    Ok(())
}

fn print_status(state: &RunState) {
    println!("run: {}", state.id);
    println!("status: {}", run_status_label(&state.status));
    println!("goal: {}", state.goal);
    println!("updated: {}", state.updated_at);

    if state.steps.is_empty() {
        println!("steps[0]:");
        println!("help[2]:");
        println!("  Run `niles show {}`", state.id);
        println!("  Run `niles status {} --json`", state.id);
        return;
    }

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
            .unwrap_or_else(|| "signal".to_owned());
        if has_roles {
            println!(
                "  {},{},{},{},{},{}",
                step.index,
                step.role.as_deref().unwrap_or("-"),
                step_kind_label(&step.kind),
                step.label,
                step_status_label(&step.status),
                exit
            );
        } else {
            println!(
                "  {},{},{},{},{}",
                step.index,
                step_kind_label(&step.kind),
                step.label,
                step_status_label(&step.status),
                exit
            );
        }
    }

    let focus_step = state
        .steps
        .iter()
        .rev()
        .find(|step| matches!(step.status, StepStatus::Failed))
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

pub fn log(selector: RunSelector, step: Option<usize>, stderr: bool, both: bool) -> Result<()> {
    let run_dir = selector.resolve()?;
    let state = read_state(&run_dir)?;
    let record = selected_step(&state, step)?;

    if both {
        print_log_file("stdout", &record.stdout)?;
        print_log_file("stderr", &record.stderr)?;
    } else if stderr {
        print!(
            "{}",
            fs::read_to_string(&record.stderr)
                .with_context(|| { format!("failed to read stderr log {}", record.stderr) })?
        );
    } else {
        print!(
            "{}",
            fs::read_to_string(&record.stdout)
                .with_context(|| { format!("failed to read stdout log {}", record.stdout) })?
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

fn create_run(spec: TaskSpec, task_file: Option<Utf8PathBuf>) -> Result<()> {
    if spec.steps.is_empty() {
        bail!("task spec must contain at least one step");
    }

    let now = Utc::now();
    let id = format!(
        "{}{:09}Z",
        now.format("%Y%m%dT%H%M%S"),
        now.timestamp_subsec_nanos()
    );
    let run_dir = Utf8Path::new(".niles").join("runs").join(&id);
    fs::create_dir_all(&run_dir).with_context(|| format!("failed to create {run_dir}"))?;
    let plan = summarize_spec(&spec);

    let state = RunState {
        id: id.clone(),
        goal: spec.goal.clone(),
        task_file,
        created_at: now,
        updated_at: now,
        status: RunStatus::Created,
        steps: Vec::new(),
    };

    let state_path = run_dir.join("state.json");
    write_state(&state_path, &state)?;

    let plan_path = run_dir.join("plan.json");
    fs::write(&plan_path, serde_json::to_string_pretty(&plan)?)
        .with_context(|| format!("failed to write {plan_path}"))?;

    println!("run: {id}");
    println!("state: {state_path}");
    execute_run(&spec, &run_dir, state, &state_path)?;

    Ok(())
}

fn execute_run(
    spec: &TaskSpec,
    run_dir: &Utf8Path,
    mut state: RunState,
    state_path: &Utf8Path,
) -> Result<()> {
    let workspace = spec.workspace.as_deref().unwrap_or(Utf8Path::new("."));
    let steps_dir = run_dir.join("steps");
    fs::create_dir_all(&steps_dir).with_context(|| format!("failed to create {steps_dir}"))?;

    state.status = RunStatus::Running;
    state.updated_at = Utc::now();
    write_state(state_path, &state)?;

    for (index, step) in spec.steps.iter().enumerate() {
        let step_number = index + 1;
        let result = match step {
            TaskStep::Agent { agent, task, role } => {
                print_step_start(step_number, role.as_deref(), "agent", agent);
                run_agent_step(
                    step_number,
                    role.clone(),
                    agent,
                    task,
                    spec,
                    workspace,
                    &steps_dir,
                )
            }
            TaskStep::Command { command, role } => {
                print_step_start(step_number, role.as_deref(), "command", command);
                run_command_step(
                    step_number,
                    role.clone(),
                    command,
                    spec,
                    workspace,
                    &steps_dir,
                )
            }
        }?;

        let failed = matches!(result.status, StepStatus::Failed);
        state.steps.push(result);
        state.status = if failed {
            RunStatus::Failed
        } else {
            RunStatus::Running
        };
        state.updated_at = Utc::now();
        write_state(state_path, &state)?;

        if failed {
            println!("status: failed");
            if let Some(step) = state.steps.last() {
                print_failure_summary(step);
            }
            bail!("step {step_number} failed");
        }
    }

    state.status = RunStatus::Completed;
    state.updated_at = Utc::now();
    write_state(state_path, &state)?;
    println!("status: completed");

    Ok(())
}

fn run_agent_step(
    step_number: usize,
    role: Option<String>,
    agent: &str,
    task: &str,
    spec: &TaskSpec,
    workspace: &Utf8Path,
    steps_dir: &Utf8Path,
) -> Result<StepRecord> {
    let config = agent_invocation(agent, spec.agents.get(agent));
    let mut args = config.args;
    let stdin = match config.prompt {
        PromptMode::Arg => {
            args.push(task.to_owned());
            None
        }
        PromptMode::Stdin => Some(task),
    };

    run_process(
        step_number,
        role,
        StepKind::Agent,
        agent,
        &config.binary,
        &args,
        stdin,
        workspace,
        steps_dir,
    )
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
    let command_line = command_config_run(config);
    run_process(
        step_number,
        role,
        StepKind::Command,
        command,
        "sh",
        &["-c".to_owned(), command_line.to_owned()],
        None,
        workspace,
        steps_dir,
    )
}

fn print_step_start(step_number: usize, role: Option<&str>, kind: &str, label: &str) {
    match role {
        Some(role) => println!("step {step_number}: {role} {kind} {label}"),
        None => println!("step {step_number}: {kind} {label}"),
    }
}

struct AgentInvocation {
    binary: String,
    args: Vec<String>,
    prompt: PromptMode,
}

fn agent_invocation(agent: &str, config: Option<&AgentConfig>) -> AgentInvocation {
    let default_binary = agent.to_owned();
    let default_args = match agent {
        "codex" => vec!["exec".to_owned()],
        "claude" => vec!["-p".to_owned()],
        _ => Vec::new(),
    };

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
            prompt: PromptMode::Arg,
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
        step_kind_label(&step.kind),
        step.label
    );
    eprintln!(
        "  exit: {}",
        step.exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_owned())
    );
    eprintln!("  stderr: {}", step.stderr);
    if let Some(diff) = &step.diff {
        eprintln!("  diff: {diff}");
    }
    eprintln!("stderr tail:");

    match stderr_tail(&step.stderr, 12) {
        Ok(lines) if lines.is_empty() => eprintln!("  <empty>"),
        Ok(lines) => {
            for line in lines {
                eprintln!("  {line}");
            }
        }
        Err(err) => eprintln!("  <failed to read stderr: {err}>"),
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
