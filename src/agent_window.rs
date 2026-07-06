use std::fs;

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::{
    agents,
    config::spec::{PromptMode, load_project_config_from},
    tmux,
    usage::UsageAttribution,
    util::slugify,
};

const UNROLE_STEP_WINDOW_SEGMENT: &str = "step";

#[derive(Clone, Copy)]
pub(crate) enum AgentWindowPrompt<'a> {
    BriefFile(&'a Utf8Path),
    Prepared { stdin: Option<&'a str> },
}

pub(crate) fn worker_window_name(id: &str) -> String {
    format!("niles-{id}")
}

pub(crate) fn worker_window_name_from_target(id: &str, target: &str) -> String {
    window_name_from_target(target).unwrap_or_else(|| worker_window_name(id))
}

fn window_name_from_target(target: &str) -> Option<String> {
    target
        .rsplit_once(':')
        .map(|(_, window)| window.to_owned())
        .filter(|window| !window.is_empty())
}

/// Self-descriptive tmux window name for a step, e.g. `niles-claude-planner-s1-9000z`.
/// The agent + role make it readable in `tmux list-windows`; the step number and
/// a short run-id tail keep it unique across repeated roles and concurrent runs.
pub(crate) fn step_window_name(
    run_id: &str,
    step_number: usize,
    role: Option<&str>,
    agent: &str,
) -> String {
    let shortid = &run_id[run_id.len().saturating_sub(6)..];
    let role = match role {
        Some(role) => role,
        None => UNROLE_STEP_WINDOW_SEGMENT,
    };
    format!(
        "niles-{}-{}-s{step_number}-{}",
        slugify(agent),
        slugify(role),
        slugify(shortid)
    )
}

pub(crate) fn legacy_step_window_name(run_id: &str, index: usize) -> String {
    format!("niles-{run_id}-s{index}")
}

/// Launch `agent` interactively in a fresh tmux window driven by `brief_path`,
/// returning its `session:window` target. Shared by workers and per-step
/// orchestration so both use the same interactive invocation and launch script.
pub(crate) fn spawn_agent_window(
    window_name: &str,
    cwd: &Utf8Path,
    agent: &str,
    project: &Utf8Path,
    brief_path: &Utf8Path,
    launch_path: &Utf8Path,
    usage_attribution: Option<&UsageAttribution>,
) -> Result<String> {
    if !brief_path.is_file() {
        bail!("cannot launch agent window {window_name}: brief does not exist at {brief_path}");
    }

    let config = load_project_config_from(project)?;
    let config = agents::config_for(&config.agents, agent)?;
    let mut invocation = agents::invocation(agent, config, agents::InvocationDefaults::Worker)?;
    if let Some(session_id) = usage_attribution.and_then(UsageAttribution::claude_session_id) {
        agents::append_session_id_arg(&mut invocation, session_id);
    }
    spawn_prepared_agent_window(
        window_name,
        cwd,
        &invocation,
        launch_path,
        AgentWindowPrompt::BriefFile(brief_path),
    )
}

pub(crate) fn spawn_prepared_agent_window(
    window_name: &str,
    cwd: &Utf8Path,
    invocation: &agents::AgentInvocation,
    launch_path: &Utf8Path,
    prompt: AgentWindowPrompt<'_>,
) -> Result<String> {
    write_launch_script(launch_path, invocation, prompt)?;
    let command = format!("sh {}", shell_quote(launch_path.as_str()));
    open_window(window_name, cwd, &command)
}

fn write_launch_script(
    path: &Utf8Path,
    invocation: &agents::AgentInvocation,
    prompt: AgentWindowPrompt<'_>,
) -> Result<()> {
    let mut body = String::new();
    body.push_str("#!/bin/sh\n");
    body.push_str("set -eu\n");
    if let AgentWindowPrompt::BriefFile(brief_path) = prompt {
        body.push_str("BRIEF=");
        body.push_str(&shell_quote(brief_path.as_str()));
        body.push('\n');
    }
    for (key, value) in &invocation.env {
        body.push_str("export ");
        body.push_str(key);
        body.push('=');
        body.push_str(&shell_assignment_value(value));
        body.push('\n');
    }
    let prepared_stdin = match prompt {
        AgentWindowPrompt::Prepared { stdin } => stdin,
        AgentWindowPrompt::BriefFile(_) => None,
    };
    if let Some(stdin) = prepared_stdin {
        write_prepared_stdin_setup(&mut body, path, stdin);
    }

    write_exec_command(&mut body, invocation);
    match prompt {
        AgentWindowPrompt::BriefFile(_) => match invocation.prompt {
            PromptMode::Arg => body.push_str(" \"$(cat \"$BRIEF\")\"\n"),
            PromptMode::Stdin => body.push_str(" < \"$BRIEF\"\n"),
        },
        AgentWindowPrompt::Prepared { stdin } => match stdin {
            Some(_) => body.push_str(" < \"$PROMPT_INPUT\"\n"),
            None => body.push('\n'),
        },
    }

    fs::write(path, body).with_context(|| format!("failed to write {path}"))
}

fn write_exec_command(body: &mut String, invocation: &agents::AgentInvocation) {
    body.push_str("exec ");
    body.push_str(&shell_quote(&invocation.binary));
    for arg in &invocation.args {
        body.push(' ');
        body.push_str(&shell_quote(arg));
    }
}

fn write_prepared_stdin_setup(body: &mut String, launch_path: &Utf8Path, stdin: &str) {
    let prompt_path = launch_path.with_extension("stdin");
    body.push_str("PROMPT_INPUT=");
    body.push_str(&shell_quote(prompt_path.as_str()));
    body.push('\n');
    body.push_str("printf '%s' ");
    body.push_str(&shell_quote(stdin));
    body.push_str(" > \"$PROMPT_INPUT\"\n");
}

pub(crate) fn capture_target(target: &str, lines: usize) -> Result<String> {
    tmux::capture_pane(target, lines)
}

/// Capture the last `lines` of a step window's pane by window name, resolving
/// the active session. Used to fold an interactive step's output into the run.
pub(crate) fn capture_window(window_name: &str, lines: usize) -> Result<String> {
    capture_target(&active_window_target(window_name)?, lines)
}

pub(crate) fn send_target(target: &str, message: &str) -> Result<()> {
    tmux::send_line(target, message)
}

pub(crate) fn send_window(window_name: &str, message: &str) -> Result<()> {
    tmux::send_line(&active_window_target(window_name)?, message)
}

/// Kill a tmux window by name in the active session. Used to tear down a worker
/// or step window once it is done.
pub(crate) fn close_window(window_name: &str) -> Result<()> {
    let session = tmux::current_or_named_session("niles")?;
    tmux::kill_window(&session, window_name)
}

pub(crate) fn target_exists(target: &str) -> Result<bool> {
    tmux::target_exists(target)
}

/// Open a detached tmux window running `command` in `cwd` and return its
/// `session:window` target.
pub(crate) fn open_window(window_name: &str, cwd: &Utf8Path, command: &str) -> Result<String> {
    let session = tmux::current_or_named_session("niles")?;
    tmux::ensure_window_available(&session, window_name)?;
    let target = format!("{session}:{window_name}");
    tmux::new_window(&session, window_name, cwd, command)?;
    Ok(target)
}

fn active_window_target(window_name: &str) -> Result<String> {
    let session = tmux::current_or_named_session("niles")?;
    Ok(format!("{session}:{window_name}"))
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn shell_assignment_value(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/'))
    {
        return value.to_owned();
    }

    shell_quote(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quotes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn worker_window_names_use_niles_prefix_and_target_suffix() {
        assert_eq!(worker_window_name("auth-fix"), "niles-auth-fix");
        assert_eq!(
            worker_window_name_from_target("auth-fix", "niles:niles-auth-fix"),
            "niles-auth-fix"
        );
        assert_eq!(
            worker_window_name_from_target("auth-fix", "legacy-target-without-session"),
            "niles-auth-fix"
        );
    }

    #[test]
    fn step_window_names_include_agent_role_step_and_run_tail() {
        assert_eq!(
            step_window_name("runabcdef", 2, Some("planner"), "claude"),
            "niles-claude-planner-s2-abcdef"
        );
        assert_eq!(
            step_window_name("runabcdef", 2, None, "codex cli"),
            "niles-codex-cli-step-s2-abcdef"
        );
        assert_eq!(
            legacy_step_window_name("runabcdef", 2),
            "niles-runabcdef-s2"
        );
    }
}
