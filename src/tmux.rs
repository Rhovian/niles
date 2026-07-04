use std::{
    env,
    ffi::OsString,
    process::{Command, ExitStatus, Output, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

const SEND_LINE_SUBMIT_DELAY: Duration = Duration::from_millis(75);
const SEND_LINE_SUBMIT_KEY: &str = "C-m";

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

fn status_with_terminal(args: &[OsString]) -> Result<ExitStatus> {
    Command::new("tmux")
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to run tmux {}", display_os_args(args)))
}

pub(crate) fn capture_pane(target: &str, lines: usize) -> Result<String> {
    let start = capture_start(lines);
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

fn capture_start(lines: usize) -> String {
    if lines == 0 {
        "-".to_owned()
    } else {
        format!("-{lines}")
    }
}

pub(crate) fn send_line(target: &str, line: &str) -> Result<()> {
    run(send_line_literal_args(target, line))?;
    // Give the worker TUI a render tick before sending the submit keystroke.
    thread::sleep(SEND_LINE_SUBMIT_DELAY);
    run(send_line_submit_args(target))
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

pub(crate) fn launch_foreground_session(
    session: &str,
    cwd: &Utf8Path,
    argv: &[OsString],
) -> Result<ExitStatus> {
    let args = foreground_new_session_args(session, cwd, argv);
    status_with_terminal(&args)
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
    let session_target = new_window_session_target(session);
    run(new_window_args(&session_target, window_name, cwd, command))
}

fn new_window_session_target(session: &str) -> String {
    format!("{session}:")
}

fn new_window_args<'a>(
    session_target: &'a str,
    window_name: &'a str,
    cwd: &'a Utf8Path,
    command: &'a str,
) -> [&'a str; 9] {
    [
        "new-window",
        "-d",
        "-t",
        session_target,
        "-n",
        window_name,
        "-c",
        cwd.as_str(),
        command,
    ]
}

pub(crate) fn kill_window(session: &str, window_name: &str) -> Result<()> {
    run(["kill-window", "-t", &format!("{session}:{window_name}")])
}

pub(crate) fn target_exists(target: &str) -> Result<bool> {
    let Some((session, window_name)) = target.rsplit_once(':') else {
        return Ok(false);
    };
    if session.is_empty() || window_name.is_empty() {
        return Ok(false);
    }

    let output = output(["list-windows", "-t", session, "-F", "#{window_name}"])
        .with_context(|| format!("failed to list tmux windows in session {session}"))?;
    if !output.status.success() {
        return Ok(false);
    }

    Ok(window_list_contains(&output.stdout, window_name))
}

fn current_session_name() -> Result<Option<String>> {
    let output =
        output(["display-message", "-p", "#S"]).context("failed to query current tmux session")?;
    if !output.status.success() {
        return Ok(None);
    }

    Ok(session_name_from_stdout(&output.stdout))
}

pub(crate) fn has_session(name: &str) -> bool {
    matches!(
        Command::new("tmux")
            .args(["has-session", "-t", name])
            .stdin(Stdio::null())
            .status(),
        Ok(status) if status.success()
    )
}

fn display_os_args(args: &[OsString]) -> String {
    args.iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
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

fn send_line_literal_args<'a>(target: &'a str, line: &'a str) -> [&'a str; 5] {
    ["send-keys", "-t", target, "-l", line]
}

fn send_line_submit_args(target: &str) -> [&str; 4] {
    ["send-keys", "-t", target, SEND_LINE_SUBMIT_KEY]
}

fn foreground_new_session_args(session: &str, cwd: &Utf8Path, argv: &[OsString]) -> Vec<OsString> {
    let mut args = ["new-session", "-s", session, "-c", cwd.as_str(), "--"]
        .map(OsString::from)
        .to_vec();
    args.extend(argv.iter().cloned());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_args_owns_argument_strings() {
        assert_eq!(
            collect_args(["send-keys", "-t", "niles:step", SEND_LINE_SUBMIT_KEY]),
            ["send-keys", "-t", "niles:step", SEND_LINE_SUBMIT_KEY].map(str::to_owned)
        );
    }

    #[test]
    fn send_line_literal_args_preserve_multiline_message_as_one_argument() {
        assert_eq!(
            send_line_literal_args("niles:step", "line 1\nline 2"),
            ["send-keys", "-t", "niles:step", "-l", "line 1\nline 2"]
        );
    }

    #[test]
    fn send_line_submit_args_use_discrete_control_m() {
        assert_eq!(
            send_line_submit_args("niles:step"),
            ["send-keys", "-t", "niles:step", SEND_LINE_SUBMIT_KEY]
        );
    }

    #[test]
    fn format_capture_trims_tmux_padding_and_restores_single_newline() {
        assert_eq!(format_capture(b"line 1\nline 2\n\n\n"), "line 1\nline 2\n");
        assert_eq!(format_capture(b"\n\n"), "");
    }

    #[test]
    fn capture_start_uses_full_history_for_zero_lines() {
        assert_eq!(capture_start(0), "-");
        assert_eq!(capture_start(2000), "-2000");
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

    #[test]
    fn new_window_args_target_session_with_trailing_colon() {
        let session_target = new_window_session_target("niles");
        let args = new_window_args(
            &session_target,
            "niles-auth-fix",
            Utf8Path::new("/tmp/workspace"),
            "sh launch.sh",
        );

        assert_eq!(
            args,
            [
                "new-window",
                "-d",
                "-t",
                "niles:",
                "-n",
                "niles-auth-fix",
                "-c",
                "/tmp/workspace",
                "sh launch.sh"
            ]
        );
    }

    #[test]
    fn foreground_new_session_args_preserve_argv_boundaries() {
        let argv = [
            "/opt/homebrew/bin/niles",
            "--goal",
            "fix quoted path /tmp/with spaces",
            "--manager",
            "codex:gpt-5:high",
        ]
        .map(OsString::from);

        let args = foreground_new_session_args(
            "niles",
            Utf8Path::new("/tmp/workspace with spaces"),
            &argv,
        );

        assert_eq!(
            args,
            [
                "new-session",
                "-s",
                "niles",
                "-c",
                "/tmp/workspace with spaces",
                "--",
                "/opt/homebrew/bin/niles",
                "--goal",
                "fix quoted path /tmp/with spaces",
                "--manager",
                "codex:gpt-5:high",
            ]
            .map(OsString::from)
        );
    }
}
