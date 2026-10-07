use std::{
    env,
    process::{Command, Output, Stdio},
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

mod home;
mod panels;
mod send;
mod session;
mod target;

pub(crate) use home::{close_view, install_home_bindings, open_home, show_in_view};
pub(crate) use panels::open_panel;
pub(crate) use session::{
    configure_status, kill_session, lead_running, open_session, project_session, switch_or_attach,
    windows,
};

pub(crate) use send::send_line;
use target::{LIVE_WINDOW_FORMAT, WindowPresence, window_presence};
pub(crate) use target::{SessionName, TargetState, TmuxTarget, WindowTarget, target_state};

/// Runs a tmux command whose output nobody reads.
///
/// Captured rather than inherited: the watcher types into the lead's pane from inside the lead's
/// own process, so anything tmux printed here would land in the middle of the TUI it is nudging.
/// tmux's own message belongs in the error either way — that is where the caller reads it.
fn run(args: &[&str]) -> Result<()> {
    let output = output(args)?;
    if !output.status.success() {
        bail!(
            "tmux {} exited with {}: {}",
            args.join(" "),
            output.status,
            normalize_stderr(&output.stderr)
        );
    }
    Ok(())
}

/// Runs a tmux command for what it prints.
fn query(args: &[&str]) -> Result<String> {
    let output = output(args)?;
    if !output.status.success() {
        bail!(
            "tmux {} exited with {}: {}",
            args.join(" "),
            output.status,
            normalize_stderr(&output.stderr)
        );
    }
    String::from_utf8(output.stdout).with_context(|| format!("tmux {} printed non-UTF-8", args[0]))
}

fn output(args: &[&str]) -> Result<Output> {
    Command::new("tmux")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run tmux {}", args.join(" ")))
}

pub(crate) fn capture_pane(target: &TmuxTarget, lines: usize) -> Result<String> {
    let start = capture_start(lines);
    let arg = target.as_str();
    capture(target, &["capture-pane", "-p", "-t", arg, "-S", &start])
}

/// Captures only the pane's currently visible screen, excluding scrollback.
pub(crate) fn capture_visible_pane(target: &TmuxTarget) -> Result<String> {
    capture(target, &["capture-pane", "-p", "-J", "-t", target.as_str()])
}

/// The visible screen with wrapped lines left unjoined, so each row lines up with `cursor_y`.
pub(crate) fn capture_visible_rows(target: &TmuxTarget) -> Result<String> {
    capture(target, &["capture-pane", "-p", "-t", target.as_str()])
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CursorPosition {
    pub x: usize,
    pub y: usize,
    pub visible: bool,
}

pub(crate) fn cursor_position(target: &TmuxTarget) -> Result<CursorPosition> {
    let value = display(target.as_str(), "#{cursor_x} #{cursor_y} #{cursor_flag}")?;
    let [x, y, flag] = value.split_whitespace().collect::<Vec<_>>()[..] else {
        bail!("unexpected tmux cursor position {value:?}");
    };
    Ok(CursorPosition {
        x: x.parse()
            .with_context(|| format!("unexpected tmux cursor position {value:?}"))?,
        y: y.parse()
            .with_context(|| format!("unexpected tmux cursor position {value:?}"))?,
        visible: flag == "1",
    })
}

fn capture(target: &TmuxTarget, args: &[&str]) -> Result<String> {
    let output =
        output(args).with_context(|| format!("failed to run tmux capture-pane for {target}"))?;

    if !output.status.success() {
        bail!(
            "tmux capture-pane failed for {target}: {}",
            normalize_stderr(&output.stderr)
        );
    }

    Ok(format_capture(&output.stdout))
}

fn display(target: &str, format: &str) -> Result<String> {
    let output = output(&["display", "-p", "-t", target, format])?;
    if !output.status.success() {
        bail!(
            "tmux display failed for {target}: {}",
            normalize_stderr(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn capture_start(lines: usize) -> String {
    if lines == 0 {
        "-".to_owned()
    } else {
        format!("-{lines}")
    }
}

/// The tmux session this process is running in.
///
/// Project leads and workers run in the session named after their registry entry and tagged with
/// the project path. Agent commands require that tmux session.
pub(crate) fn current_session() -> Result<SessionName> {
    if env::var_os("TMUX").is_none() {
        bail!(
            "niles agent commands must run inside tmux, and this process has no $TMUX. If you are \
             a niles lead or worker, your agent session was moved out of its tmux pane (for \
             example into a background session): resume it in its pane. Otherwise run bare \
             `niles` to open a project session."
        );
    }

    let output =
        output(&["display-message", "-p", "#S"]).context("failed to query current tmux session")?;
    if !output.status.success() {
        bail!(
            "tmux display-message failed: {}",
            normalize_stderr(&output.stderr)
        );
    }
    SessionName::new(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .context("failed to determine the current tmux session name")
}

fn window_presence_in(session: &SessionName, window: &str) -> Result<WindowPresence> {
    let output = output(&[
        "list-windows",
        "-t",
        &target::exact(session.as_str()),
        "-F",
        LIVE_WINDOW_FORMAT,
    ])
    .with_context(|| format!("failed to list tmux windows in session {session}"))?;
    if !output.status.success() {
        bail!(
            "tmux list-windows failed for session {session}: {}",
            normalize_stderr(&output.stderr)
        );
    }
    Ok(window_presence(&output.stdout, window))
}

pub(crate) fn ensure_window_available(session: &SessionName, window_name: &str) -> Result<()> {
    if window_presence_in(session, window_name)? != WindowPresence::Absent {
        bail!("tmux window {session}:{window_name} already exists");
    }
    Ok(())
}

pub(crate) fn new_window(
    session: &SessionName,
    window_name: &str,
    cwd: &Utf8Path,
    command: &str,
) -> Result<()> {
    let session_target = new_window_session_target(session);
    run(&[
        "new-window",
        "-d",
        "-t",
        &session_target,
        "-n",
        window_name,
        "-c",
        cwd.as_str(),
        command,
    ])
}

fn new_window_session_target(session: &SessionName) -> String {
    format!("{}:", target::exact(session.as_str()))
}

pub(crate) fn kill_window(target: &WindowTarget) -> Result<()> {
    run(&["kill-window", "-t", &target.target_arg()])
}

pub(crate) fn set_window_option(target: &WindowTarget, option: &str, value: &str) -> Result<()> {
    run(&[
        "set-option",
        "-w",
        "-t",
        &target.target_arg(),
        option,
        value,
    ])
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

fn normalize_stderr(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn new_windows_target_the_session_with_a_trailing_colon() {
        let session = SessionName::new("niles").unwrap();
        assert_eq!(new_window_session_target(&session), "=niles:");
    }
}
