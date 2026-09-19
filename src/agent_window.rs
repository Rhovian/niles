use std::fs;

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::{
    agents,
    config::spec::{PromptMode, load_project_config_from},
    tmux::{self, SessionName, WindowTarget},
    wake,
};

pub(crate) fn worker_window_name(id: &str) -> String {
    format!("niles-{id}")
}

/// The files a worker window is launched from and reports into.
pub(crate) struct WorkerPaths<'a> {
    pub(crate) brief: &'a Utf8Path,
    pub(crate) launch: &'a Utf8Path,
    pub(crate) status: &'a Utf8Path,
}

pub(crate) fn spawn_agent_window_in_session(
    session: &SessionName,
    window_name: &str,
    cwd: &Utf8Path,
    agent: &str,
    project: &Utf8Path,
    paths: &WorkerPaths<'_>,
) -> Result<WindowTarget> {
    let WorkerPaths {
        brief: brief_path,
        launch: launch_path,
        status: status_path,
    } = *paths;
    if !brief_path.is_file() {
        bail!("cannot launch agent window {window_name}: brief does not exist at {brief_path}");
    }

    let config = load_project_config_from(project)?;
    let config = agents::config_for(&config.agents, agent)?;
    let invocation = agents::invocation(agent, config, agents::InvocationDefaults::Worker)?;
    spawn_prepared_agent_window_in_session(
        session,
        window_name,
        cwd,
        &invocation,
        launch_path,
        brief_path,
        status_path,
    )
}

pub(crate) fn spawn_prepared_agent_window_in_session(
    session: &SessionName,
    window_name: &str,
    cwd: &Utf8Path,
    invocation: &agents::AgentInvocation,
    launch_path: &Utf8Path,
    brief_path: &Utf8Path,
    status_path: &Utf8Path,
) -> Result<WindowTarget> {
    write_launch_script(launch_path, invocation, brief_path, status_path)?;
    let command = format!("sh {}", shell_quote(launch_path.as_str()));
    open_window_in_session(session, window_name, cwd, &command)
}

/// Writes the script the worker window runs.
///
/// The agent is run, not `exec`ed. Exec replaced the shell with the agent, so when the agent died
/// there was nothing left to say so — the window simply vanished and any waiting lead learned
/// nothing until it noticed the window was gone. Running it as a child lets the script report the
/// exit as an ordinary status line, through the same cursor as every other wake.
fn write_launch_script(
    path: &Utf8Path,
    invocation: &agents::AgentInvocation,
    brief_path: &Utf8Path,
    status_path: &Utf8Path,
) -> Result<()> {
    let mut body = String::new();
    body.push_str("#!/bin/sh\n");
    body.push_str("set -eu\n");
    body.push_str("BRIEF=");
    body.push_str(&shell_quote(brief_path.as_str()));
    body.push('\n');
    body.push_str("STATUS=");
    body.push_str(&shell_quote(status_path.as_str()));
    body.push('\n');
    for (key, value) in &invocation.env {
        body.push_str("export ");
        body.push_str(key);
        body.push('=');
        body.push_str(&shell_assignment_value(value));
        body.push('\n');
    }
    body.push_str("code=0\n");
    write_agent_command(&mut body, invocation);
    match invocation.prompt {
        PromptMode::Arg => body.push_str(" \"$(cat \"$BRIEF\")\""),
        PromptMode::Stdin => body.push_str(" < \"$BRIEF\""),
    }
    // `|| code=$?` rather than a bare call: `set -e` would otherwise abort the script on a failing
    // agent, which is precisely the case the report below exists for.
    body.push_str(" || code=$?\n");
    // Literal halves single-quoted, `$code` double-quoted between them: the message expands the
    // exit status without the rest of it being subject to expansion.
    body.push_str("echo ");
    body.push_str(&shell_quote(&wake::line(
        wake::WakeKind::Closed,
        "agent exited (status ",
    )));
    body.push_str("\"$code\"");
    body.push_str(&shell_quote(")"));
    body.push_str(" >> \"$STATUS\"\n");

    fs::write(path, body).with_context(|| format!("failed to write {path}"))
}

fn write_agent_command(body: &mut String, invocation: &agents::AgentInvocation) {
    body.push_str(&shell_quote(&invocation.binary));
    for arg in &invocation.args {
        body.push(' ');
        body.push_str(&shell_quote(arg));
    }
}

pub(crate) fn capture_target(target: &WindowTarget, lines: usize) -> Result<String> {
    tmux::capture_pane(target, lines)
}

pub(crate) fn send_target(target: &WindowTarget, message: &str) -> Result<()> {
    tmux::send_line(target, message)
}

pub(crate) fn close_target(target: &WindowTarget) -> Result<()> {
    tmux::kill_window(target)
}

pub(crate) fn open_window_in_session(
    session: &SessionName,
    window_name: &str,
    cwd: &Utf8Path,
    command: &str,
) -> Result<WindowTarget> {
    tmux::ensure_window_available(session, window_name)?;
    let target = WindowTarget::new(session.clone(), window_name.to_owned())?;
    tmux::new_window(session, window_name, cwd, command)?;
    Ok(target)
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

    fn script_for(prompt: PromptMode) -> String {
        // A distinct path per call: these tests run in parallel and would otherwise delete the
        // file out from under each other.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let invocation = agents::AgentInvocation {
            binary: "codex".to_owned(),
            args: vec!["--flag".to_owned()],
            prompt,
            env: Vec::new(),
            spec: agents::parse_spec("codex").unwrap(),
        };
        let dir = std::env::temp_dir();
        let path = Utf8Path::from_path(&dir)
            .unwrap()
            .join(format!("niles-launch-test-{nanos}.sh"));
        write_launch_script(
            &path,
            &invocation,
            Utf8Path::new("/w/brief.md"),
            Utf8Path::new("/w/status.log"),
        )
        .unwrap();
        let body = fs::read_to_string(&path).unwrap();
        fs::remove_file(&path).unwrap();
        body
    }

    /// The agent is run, not `exec`ed, so something survives it to report the exit.
    #[test]
    fn the_launch_script_reports_the_agents_exit() {
        let script = script_for(PromptMode::Arg);

        assert!(!script.contains("exec "), "agent must not replace the shell:\n{script}");
        // `|| code=$?` and not a bare call: `set -e` would abort before the report otherwise.
        assert!(script.contains("|| code=$?"), "{script}");
        assert!(
            script.contains(r#"echo 'closed: agent exited (status '"$code"')' >> "$STATUS""#),
            "the status must expand, and the rest of the line must not:\n{script}"
        );
    }

    #[test]
    fn the_exit_report_follows_a_stdin_prompt_too() {
        let script = script_for(PromptMode::Stdin);

        assert!(script.contains(r#"< "$BRIEF" || code=$?"#), "{script}");
        assert!(script.contains(">> \"$STATUS\""), "{script}");
    }

    #[test]
    fn shell_quotes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn worker_window_names_use_niles_prefix() {
        assert_eq!(worker_window_name("auth-fix"), "niles-auth-fix");
    }
}
