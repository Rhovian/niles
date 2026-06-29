use std::{collections::BTreeMap, fs};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::{
    config::spec::{
        TaskSpec, TaskStep, apply_project_config, load_project_config, load_task, summarize_spec,
    },
    state::{RunState, RunStatus, StepKind, StepRecord, StepStatus},
    store::{read_state, write_state},
    util::{timestamp_id, write_json_pretty},
};

use super::{RunSelector, exec, report};

pub(crate) fn ask(agent: String, prompt: Vec<String>) -> Result<()> {
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

pub(crate) fn run(task: Utf8PathBuf, watch: bool, prepare: bool) -> Result<()> {
    let spec = with_project_config(load_task(&task)?)?;
    if prepare {
        prepare_run(spec, Some(task))
    } else {
        create_run(spec, Some(task), watch)
    }
}

pub(crate) fn run_manifest(task: Utf8PathBuf, watch: bool) -> Result<()> {
    let spec = with_project_config(load_task(&task)?)?;
    create_run(spec, Some(task), watch)
}

pub(crate) fn resume(selector: RunSelector, watch: bool) -> Result<()> {
    let run_dir = selector.resolve()?;
    let state_path = run_dir.join("state.json");
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

pub(in crate::runner) fn with_project_config(spec: TaskSpec) -> Result<TaskSpec> {
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
        let failed = exec::execute_single_step(
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
                report::print_failure_summary(step);
            }
            bail!("step {step_number} failed");
        }
    }

    state.status = RunStatus::Completed;
    state.updated_at = Utc::now();
    write_state(state_path, &state)?;
    if watch {
        report::print_watch_snapshot(&state);
    }
    println!("status: completed");

    Ok(())
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
