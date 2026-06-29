use std::{
    env,
    process::{Command, Output, Stdio},
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

fn collect_args<I, S>(args: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect()
}

pub(crate) fn run<I, S>(args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = collect_args(args);
    let status = Command::new("tmux")
        .args(&args)
        .stdin(Stdio::null())
        .status()
        .with_context(|| format!("failed to run tmux {}", args.join(" ")))?;
    if !status.success() {
        bail!("tmux {} exited with {status}", args.join(" "));
    }
    Ok(())
}

fn output<I, S>(args: I) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = collect_args(args);
    Command::new("tmux")
        .args(&args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run tmux {}", args.join(" ")))
}

pub(crate) fn capture_pane(target: &str, lines: usize) -> Result<String> {
    let start = format!("-{lines}");
    let output = output(["capture-pane", "-p", "-t", target, "-S", &start])
        .with_context(|| format!("failed to run tmux capture-pane for {target}"))?;

    if !output.status.success() {
        bail!(
            "tmux capture-pane failed for {target}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(format_capture(&output.stdout))
}

pub(crate) fn send_line(target: &str, line: &str) -> Result<()> {
    run(["send-keys", "-t", target, "-l", line])?;
    run(["send-keys", "-t", target, "Enter"])
}

pub(crate) fn current_or_named_session(name: &str) -> Result<String> {
    if env::var_os("TMUX").is_some()
        && let Some(session) = current_session_name()?
    {
        return Ok(session);
    }

    if has_session(name) {
        Ok(name.to_owned())
    } else {
        run(["new-session", "-d", "-s", name])?;
        Ok(name.to_owned())
    }
}

pub(crate) fn ensure_window_available(session: &str, window_name: &str) -> Result<()> {
    let output = output(["list-windows", "-t", session, "-F", "#{window_name}"])
        .with_context(|| format!("failed to list tmux windows in session {session}"))?;

    if !output.status.success() {
        bail!(
            "tmux list-windows failed for session {session}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    if window_list_contains(&output.stdout, window_name) {
        bail!("tmux window {session}:{window_name} already exists");
    }

    Ok(())
}

pub(crate) fn new_window(
    session: &str,
    window_name: &str,
    cwd: &Utf8Path,
    command: &str,
) -> Result<()> {
    run([
        "new-window",
        "-d",
        "-t",
        session,
        "-n",
        window_name,
        "-c",
        cwd.as_str(),
        command,
    ])
}

pub(crate) fn kill_window(session: &str, window_name: &str) -> Result<()> {
    run(["kill-window", "-t", &format!("{session}:{window_name}")])
}

fn current_session_name() -> Result<Option<String>> {
    let output =
        output(["display-message", "-p", "#S"]).context("failed to query current tmux session")?;
    if !output.status.success() {
        return Ok(None);
    }

    Ok(session_name_from_stdout(&output.stdout))
}

fn has_session(name: &str) -> bool {
    matches!(
        Command::new("tmux")
            .args(["has-session", "-t", name])
            .stdin(Stdio::null())
            .status(),
        Ok(status) if status.success()
    )
}

fn format_capture(stdout: &[u8]) -> String {
    // tmux pads the capture to the pane height; drop the trailing blank lines.
    let text = String::from_utf8_lossy(stdout);
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

fn window_list_contains(stdout: &[u8], window_name: &str) -> bool {
    String::from_utf8_lossy(stdout)
        .lines()
        .any(|line| line == window_name)
}

fn session_name_from_stdout(stdout: &[u8]) -> Option<String> {
    let session = String::from_utf8_lossy(stdout).trim().to_owned();
    (!session.is_empty()).then_some(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_args_owns_argument_strings() {
        assert_eq!(
            collect_args(["send-keys", "-t", "niles:step", "Enter"]),
            ["send-keys", "-t", "niles:step", "Enter"].map(str::to_owned)
        );
    }

    #[test]
    fn format_capture_trims_tmux_padding_and_restores_single_newline() {
        assert_eq!(format_capture(b"line 1\nline 2\n\n\n"), "line 1\nline 2\n");
        assert_eq!(format_capture(b"\n\n"), "");
    }

    #[test]
    fn window_list_contains_matches_exact_window_names() {
        let output = b"niles-run\nniles-run-extra\n";

        assert!(window_list_contains(output, "niles-run"));
        assert!(!window_list_contains(output, "run"));
    }

    #[test]
    fn session_name_from_stdout_trims_and_ignores_empty_output() {
        assert_eq!(
            session_name_from_stdout(b"niles\n"),
            Some("niles".to_owned())
        );
        assert_eq!(session_name_from_stdout(b" \n"), None);
    }
}
