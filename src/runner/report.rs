use std::{
    fs,
    io::{self, Write},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::{
    process::{exit_code_label, role_prefix},
    schema::{self, ArtifactKind},
    state::{RunState, RunStatus, StepRecord, StepStatus},
    store::{read_state, selected_step, state_path},
};

use super::RunSelector;

pub(crate) fn status(selector: RunSelector, json: bool) -> Result<()> {
    let run_dir = selector.resolve()?;
    let path = state_path(&run_dir);
    if json {
        let state = schema::read_json_value_as::<RunState>(&path, ArtifactKind::RunState)?;
        let state = serde_json::to_string_pretty(&state).context("failed to format run state")?;
        println!("{state}");
    } else {
        let state = read_state(&run_dir)?;
        print_status(&state);
    }
    Ok(())
}

pub(crate) fn watch(selector: RunSelector, interval: f64, no_clear: bool) -> Result<()> {
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

pub(crate) fn show(selector: RunSelector) -> Result<()> {
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
            "  {}. {}{} {} {}{}{}{}",
            step.index,
            role_prefix(step.role.as_deref()),
            step.kind,
            step.label,
            step.status,
            step.exit_code
                .map(|code| format!(" ({code})"))
                .unwrap_or_default(),
            step.context
                .as_ref()
                .map(|path| format!(" context {path}"))
                .unwrap_or_default(),
            agent_tier_suffix(step)
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
    print_agent_tiers(state);

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

pub(in crate::runner) fn print_watch_snapshot(state: &RunState) {
    println!("watch:");
    println!("run: {}", state.id);
    println!("status: {}", state.status);
    println!("updated: {}", state.updated_at);

    if state.steps.is_empty() {
        println!("steps[0]:");
    } else {
        print_steps_table(state);
        print_agent_tiers(state);
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

fn print_agent_tiers(state: &RunState) {
    let tiered_steps = state
        .steps
        .iter()
        .filter(|step| step.model.is_some() || step.effort.is_some())
        .collect::<Vec<_>>();
    if tiered_steps.is_empty() {
        return;
    }

    println!(
        "agent_tiers[{}]{{index,agent_family,model,effort}}:",
        tiered_steps.len()
    );
    for step in tiered_steps {
        println!(
            "  {},{},{},{}",
            step.index,
            step.agent_family.as_deref().unwrap_or("-"),
            step.model.as_deref().unwrap_or("-"),
            step.effort.as_deref().unwrap_or("-")
        );
    }
}

fn agent_tier_suffix(step: &StepRecord) -> String {
    if step.model.is_none() && step.effort.is_none() {
        return String::new();
    }

    format!(
        " agent_family {} model {} effort {}",
        step.agent_family.as_deref().unwrap_or("-"),
        step.model.as_deref().unwrap_or("-"),
        step.effort.as_deref().unwrap_or("-")
    )
}

pub(crate) fn log(
    selector: RunSelector,
    step: Option<usize>,
    stderr: bool,
    both: bool,
) -> Result<()> {
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

pub(crate) fn diff(selector: RunSelector, step: Option<usize>) -> Result<()> {
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

fn print_log_file(label: &str, path: &Utf8Path) -> Result<()> {
    println!("==> {label}: {path} <==");
    print!(
        "{}",
        fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?
    );
    Ok(())
}

pub(in crate::runner) fn print_failure_summary(step: &StepRecord) {
    eprintln!("failure:");
    eprintln!(
        "  step: {} {}{} {}",
        step.index,
        role_prefix(step.role.as_deref()),
        step.kind,
        step.label
    );
    eprintln!("  exit: {}", exit_code_label(step.exit_code));
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
