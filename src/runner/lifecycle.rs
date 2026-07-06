use std::fs;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::{
    agents,
    config::spec::{
        TaskSpec, TaskStep, apply_project_config, load_project_config, load_project_config_from,
        load_task, save_task, summarize_spec,
    },
    process::role_prefix,
    state::{RunState, RunStatus, StepKind, StepRecord, StepStatus},
    store::{read_state, state_path, write_state},
    util::{
        absolute_existing_dir, absolute_existing_file, absolute_path, absolute_path_from,
        current_dir_utf8, timestamp_id, write_json_pretty,
    },
    workspace_manifest,
};

use super::{IMPLICIT_TASK_WORKSPACE, RunSelector, spec_workspace};

pub(crate) fn run(task: Utf8PathBuf, allow_cli_mismatch: bool) -> Result<()> {
    let task = absolute_existing_file(&task, "task")?;
    let spec = with_project_config(load_task(&task)?)?;
    agents::capabilities::validate_task_agents(&spec)?;
    agents::version::preflight_task_agents(&spec, allow_cli_mismatch)?;
    prepare_run(spec, Some(task))
}

pub(crate) fn resume(selector: RunSelector) -> Result<()> {
    let run_dir = selector.resolve()?;
    let state_path = run_dir.join("state.json");
    let mut state = read_state(&run_dir)?;

    if matches!(state.status, RunStatus::Completed) {
        println!("run: {}", state.id);
        println!("status: completed");
        println!("next: nothing to resume");
        return Ok(());
    }

    let spec = load_spec_for_run(&state)?;
    let resume_from = first_incomplete_step(&state).context("run has no incomplete steps")?;
    validate_resume_shape(&state, &spec)?;
    reset_steps_from(&mut state, &spec, resume_from)?;
    if matches!(state.status, RunStatus::Failed) {
        state.status = RunStatus::Running;
    }
    state.updated_at = Utc::now();
    write_state(&state_path, &state)?;

    println!("resume: {}", state.id);
    println!("state: {state_path}");
    println!("from_step: {resume_from}");
    println!("watch: niles watch {}", state.id);
    println!("show: niles show {}", state.id);
    print_step_commands(&state, resume_from);
    Ok(())
}

pub(in crate::runner) fn with_project_config(spec: TaskSpec) -> Result<TaskSpec> {
    let config_root = current_dir_utf8()?;
    let config = load_project_config()?;
    let spec = apply_project_config(spec, config);
    with_normalized_workspace(spec, &config_root)
}

pub(in crate::runner) fn load_spec_for_run(state: &RunState) -> Result<TaskSpec> {
    let task_file = state
        .task_file
        .clone()
        .context("run has no task file; only task-backed runs support this command")?;
    let config_root = state
        .config_root
        .as_deref()
        .or_else(|| task_file.parent())
        .map_or(Utf8Path::new(IMPLICIT_TASK_WORKSPACE), |root| root);
    let config = load_project_config_from(config_root)?;
    let mut spec = apply_project_config(load_task(&task_file)?, config);
    if let Some(workspace) = &state.workspace {
        spec.workspace = Some(workspace.clone());
    }
    with_normalized_workspace(spec, config_root)
}

fn with_normalized_workspace(mut spec: TaskSpec, config_root: &Utf8Path) -> Result<TaskSpec> {
    let config_root = absolute_path(config_root)?;
    let workspace = absolute_path_from(&config_root, spec_workspace(&spec));
    let resolved_workspace = absolute_existing_dir(&workspace, "workspace")?;
    spec.workspace = Some(resolved_workspace.clone());

    if workspace_manifest::task_uses_role_bindings(&spec) {
        let manifest = workspace_manifest::load_required(&resolved_workspace)?;
        workspace_manifest::resolve_task_roles(spec, &manifest)
    } else {
        Ok(spec)
    }
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
    let workspace = spec
        .workspace
        .clone()
        .context("run spec has no resolved workspace")?;
    let config_root = current_dir_utf8()?;
    let run_dir = crate::store::workspace_run_dir(&workspace, &id)?;
    fs::create_dir_all(&run_dir).with_context(|| format!("failed to create {run_dir}"))?;
    let plan = summarize_spec(spec);

    let state = RunState {
        id,
        goal: spec.goal.clone(),
        workspace: Some(workspace.clone()),
        config_root: Some(config_root),
        task_file,
        created_at: now,
        updated_at: now,
        status: RunStatus::Created,
        steps: planned_steps(spec)?,
    };

    let state_path = run_dir.join("state.json");
    write_state(&state_path, &state)?;

    let plan_path = run_dir.join("plan.json");
    write_json_pretty(&plan_path, &plan)?;
    crate::store::register_run_location(&state.id, &workspace, &run_dir)?;

    Ok((run_dir, state, state_path))
}

/// Create a run without executing it, so the foreground manager can drive the
/// steps one at a time via `niles step` or `niles exec-step`.
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
            role_prefix(step.role.as_deref()),
            step.kind,
            step.label
        );
    }
    println!("watch: niles watch {}", state.id);
    println!("show: niles show {}", state.id);
    print_step_commands(&state, 1);

    Ok(())
}

fn print_step_commands(state: &RunState, step_index: usize) {
    let recommended = state
        .steps
        .iter()
        .find(|step| step.index == step_index)
        .map(|step| match step.kind {
            StepKind::Agent => format!("niles step {} --index {step_index}", state.id),
            StepKind::Command => format!("niles exec-step {} {step_index}", state.id),
        })
        .unwrap_or_else(|| format!("niles step {} --index {step_index}", state.id));

    println!("next: {recommended}");
    println!("step: niles step {} --index {step_index}", state.id);
    println!("exec-step: niles exec-step {} {step_index}", state.id);
    println!("wait: niles wait {} --index {step_index}", state.id);
    println!(
        "step-close: niles step-close {} --index {step_index}",
        state.id
    );
}

/// Append a step to an existing run so the manager can extend the
/// worker <verification> <-> reviewer loop until CONSENSUS or ESCALATE. The
/// step is added to both the run's task spec and its state, and a terminal run
/// is reopened to `running`.
pub(crate) fn step_add(
    selector: RunSelector,
    agent: Option<String>,
    command: Option<String>,
    role: Option<String>,
    task: Vec<String>,
) -> Result<()> {
    let run_dir = selector.resolve()?;
    let state_path = state_path(&run_dir);
    let mut state = read_state(&run_dir)?;
    let task_file = state
        .task_file
        .clone()
        .context("run has no task file; only task-backed runs support step-add")?;

    let new_step = match (agent, command) {
        (Some(agent), None) => {
            if task.is_empty() {
                bail!("step-add --agent requires task text");
            }
            TaskStep::Agent {
                agent,
                task: task.join(" "),
                role,
            }
        }
        (None, Some(command)) => TaskStep::Command { command, role },
        (Some(_), Some(_)) => bail!("step-add takes either --agent or --command, not both"),
        (None, None) => bail!("step-add requires --agent <id> or --command <name>"),
    };

    let mut spec = load_task(&task_file)?;
    let index = state.steps.len() + 1;
    if let TaskStep::Agent { agent, .. } = &new_step {
        let validation_spec = load_spec_for_run(&state)?;
        let workspace = state
            .workspace
            .as_deref()
            .context("run state has no workspace")?;
        agents::capabilities::validate_task_agent(workspace, &validation_spec.agents, agent)?;
    }
    let record = planned_step(index, &new_step)?;

    spec.steps.push(new_step.clone());
    save_task(&task_file, &spec)?;

    state.steps.push(record);
    if matches!(state.status, RunStatus::Completed | RunStatus::Failed) {
        state.status = RunStatus::Running;
    }
    state.updated_at = Utc::now();
    write_state(&state_path, &state)?;

    let record = &state.steps[index - 1];
    println!(
        "added: step {index} {}{} {}",
        role_prefix(record.role.as_deref()),
        record.kind,
        record.label
    );
    match record.kind {
        StepKind::Agent => println!("next: niles step {} --index {index}", state.id),
        StepKind::Command => println!("next: niles exec-step {} {index}", state.id),
    }
    Ok(())
}

fn planned_steps(spec: &TaskSpec) -> Result<Vec<StepRecord>> {
    spec.steps
        .iter()
        .enumerate()
        .map(|(index, step)| planned_step(index + 1, step))
        .collect()
}

fn planned_step(index: usize, step: &TaskStep) -> Result<StepRecord> {
    let (role, kind, label, agent_family, model, effort) = match step {
        TaskStep::Agent { agent, role, .. } => {
            let spec = agents::parse_spec(agent)?;
            let (agent_family, model, effort) = match spec.tier() {
                Some(tier) => (Some(tier.family), tier.model, tier.effort),
                None => (None, None, None),
            };
            (
                role.clone(),
                StepKind::Agent,
                agent.clone(),
                agent_family,
                model,
                effort,
            )
        }
        TaskStep::Command { command, role } => (
            role.clone(),
            StepKind::Command,
            command.clone(),
            None,
            None,
            None,
        ),
        TaskStep::Role { role, task } => (
            Some(role.clone()),
            if task.is_some() {
                StepKind::Agent
            } else {
                StepKind::Command
            },
            role.clone(),
            None,
            None,
            None,
        ),
    };

    Ok(StepRecord {
        index,
        role,
        kind,
        label,
        agent_family,
        model,
        effort,
        status: StepStatus::Pending,
        started_at: None,
        finished_at: None,
        exit_code: None,
        stdout: None,
        stderr: None,
        diff: None,
        context: None,
        window: None,
    })
}

fn first_incomplete_step(state: &RunState) -> Option<usize> {
    state
        .steps
        .iter()
        .find(|step| !matches!(step.status, StepStatus::Completed))
        .map(|step| step.index)
}

fn validate_resume_shape(state: &RunState, spec: &TaskSpec) -> Result<()> {
    let planned = planned_steps(spec)?;
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
    for planned in planned_steps(spec)?
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
