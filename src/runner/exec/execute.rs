use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::{
    agents,
    config::spec::{PromptMode, TaskSpec, TaskStep},
    context::{agent_prompt, write_agent_context},
    process::{ProcessSpec, run_process},
    state::{RunState, RunStatus, StepKind, StepRecord, StepStatus},
    store::write_state,
};

use super::super::report;
use super::run::{mark_step_running, step_record_position, update_step_record};

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

    let record_position = step_record_position(state, step_number)?;
    mark_step_running(&mut state.steps[record_position], context.clone());
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
