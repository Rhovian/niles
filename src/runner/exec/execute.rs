use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::{
    agents,
    config::spec::{PromptMode, TaskSpec, TaskStep},
    context::{agent_prompt, write_agent_context},
    process::{run_process, step_meta_path, ProcessSpec},
    state::{RunState, RunStatus, StepKind, StepRecord, StepStatus},
    store::write_state,
    usage::{self, UsageAgent, UsageSnapshotInput, UsageSubject},
    util::write_json_pretty,
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
                run_id: &state.id,
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
    run_id: &'a str,
    spec: &'a TaskSpec,
    workspace: &'a Utf8Path,
    steps_dir: &'a Utf8Path,
    context_path: Option<Utf8PathBuf>,
}

fn run_agent_step(step: AgentStep<'_>) -> Result<StepRecord> {
    let config = agents::config_for(&step.spec.agents, step.agent)?;
    let mut config = agents::invocation(step.agent, config, agents::InvocationDefaults::Default)?;
    let launched_at = Utc::now();
    let usage_attribution =
        usage::attribution_for_family(config.spec.family(), step.workspace, launched_at, Some(1));
    if let Some(session_id) = usage_attribution.claude_session_id() {
        agents::append_session_id_arg(&mut config, session_id);
    }
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
        role: step.role.clone(),
        kind: StepKind::Agent,
        label: step.agent,
        binary: &config.binary,
        args: &args,
        stdin: stdin.as_deref(),
        workspace: step.workspace,
        steps_dir: step.steps_dir,
        context_path: step.context_path.clone(),
    })?;
    apply_agent_tier(&mut record, &config.spec);
    record.usage_attribution = Some(usage_attribution.clone());
    let usage_path = usage::step_usage_path(step.steps_dir, step.step_number, step.agent);
    let finished_at = match record.finished_at {
        Some(finished_at) => finished_at,
        None => Utc::now(),
    };
    record.usage = usage::snapshot_usage(UsageSnapshotInput {
        subject: UsageSubject::RunStep {
            run_id: step.run_id.to_owned(),
            index: step.step_number,
            role: step.role.clone(),
            label: step.agent.to_owned(),
        },
        agent: UsageAgent {
            spec: step.agent.to_owned(),
            family: Some(config.spec.family().to_owned()),
            model: config.spec.model().map(str::to_owned),
            effort: config.spec.effort().map(str::to_owned),
        },
        attribution: Some(usage_attribution),
        started_at: record.started_at,
        finished_at,
        output_path: usage_path,
    });
    write_json_pretty(
        &step_meta_path(step.steps_dir, step.step_number, step.agent),
        &record,
    )?;
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
