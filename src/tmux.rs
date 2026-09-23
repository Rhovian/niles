use std::{
    env,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

mod target;

pub(crate) use target::{PaneTarget, SessionName, TargetState, WindowTarget, target_state};

const SEND_LINE_SUBMIT_KEY: &str = "C-m";

/// How often the pane is re-captured while watching a send land.
const SEND_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How long the pane must hold still before the paste counts as ingested.
///
/// A TUI does not redraw once per keystroke, it redraws in bursts: a 6KB message handed to hermes
/// moved the pane at 69ms, 193ms, 313ms, 554ms, 799ms and 923ms, with up to 245ms of stillness
/// between bursts. "Unchanged since the last poll" is therefore not quiet — it is the gap between
/// two bursts, and a submit sent into one is the swallowed submit this whole dance exists to
/// avoid. The window is wider than the widest observed gap, and it is a floor on how long to keep
/// looking rather than a guess at how long ingestion takes: the wait ends when the pane stops
/// moving, however long that takes.
const SEND_QUIET_WINDOW: Duration = Duration::from_millis(500);

/// Longest wait for a pasted message to render and go quiet before the submit key is sent.
/// Bounded because a pane with an animation on it never goes quiet, and a send must not hang.
const SEND_SETTLE_TIMEOUT: Duration = Duration::from_secs(6);

/// Longest wait for the pane to change after the submit key. A pane that never changes is what a
/// swallowed submit looks like, and reporting that is the whole point of watching.
const SEND_SUBMIT_TIMEOUT: Duration = Duration::from_secs(3);

/// Pane rows captured while watching a send land. The composer sits at the bottom of the pane, so
/// this only has to be deep enough to see it fill and empty.
const SEND_WATCH_LINES: usize = 50;

fn collect_args<I, S>(args: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect()
}

/// Runs a tmux command whose output nobody reads.
///
/// Captured rather than inherited: the watcher types into the lead's pane from inside the lead's
/// own process, so anything tmux printed here would land in the middle of the TUI it is nudging.
/// tmux's own message belongs in the error either way — that is where the caller reads it.
fn run<I, S>(args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = collect_args(args);
    let output = output(&args)?;
    if !output.status.success() {
        bail!(
            "tmux {} exited with {}: {}",
            args.join(" "),
            output.status,
            target::normalize_stderr(&output.stderr)
        );
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

pub(crate) fn capture_pane(target: &PaneTarget, lines: usize) -> Result<String> {
    let start = capture_start(lines);
    let arg = target.as_str();
    let output = output(["capture-pane", "-p", "-t", arg, "-S", &start])
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

/// Sends a message to a pane and confirms it left the composer.
///
/// The text and the submit key are two separate tmux calls, and a TUI still ingesting a few KB of
/// pasted text swallows a `C-m` that arrives mid-reflow: the message then sits unsent in the
/// composer while niles reports success, which is the one direction this must never fail in. So
/// the pane is watched rather than timed. The paste is given until it renders and goes quiet
/// before the submit is sent, and the pane must change after it — a submit that changes nothing
/// is an error, not a `sent:`.
pub(crate) fn send_line(target: &PaneTarget, line: &str) -> Result<()> {
    let arg = target.as_str();
    let before = capture_pane(target, SEND_WATCH_LINES)?;
    run(send_line_literal_args(arg, line))?;
    let staged = settle_pane(target, &before)?;
    run(send_line_submit_args(arg))?;
    confirm_submit_took(target, &staged)
}

/// Waits for the paste to render and the pane to go quiet, and returns what it settled on.
///
/// Neither way of giving up is an error. A pane that never changed may simply not echo what it is
/// handed, and one that never goes quiet is an agent already doing something; in both cases the
/// submit is still worth sending, and [`confirm_submit_took`] is the judge of whether it took.
fn settle_pane(target: &PaneTarget, before: &str) -> Result<String> {
    let deadline = Instant::now() + SEND_SETTLE_TIMEOUT;
    let mut previous = before.to_owned();
    let mut rendered = false;
    let mut quiet_since = None;
    loop {
        thread::sleep(SEND_POLL_INTERVAL);
        let current = capture_pane(target, SEND_WATCH_LINES)?;
        if current == previous {
            let since = quiet_since.get_or_insert_with(Instant::now);
            if rendered && since.elapsed() >= SEND_QUIET_WINDOW {
                return Ok(current);
            }
        } else {
            rendered = true;
            quiet_since = None;
            previous = current;
        }
        if Instant::now() >= deadline {
            return Ok(previous);
        }
    }
}

/// Fails unless the pane changes after the submit key.
///
/// The composer emptying, the message appearing in the transcript, the agent starting to think —
/// any of them move the pane. Nothing moving is the observed failure: the text still sitting in
/// the composer, with the worker idle and niles about to call it sent. This is only as good as
/// the quiet `staged` was captured in — against a pane that was still moving when
/// [`settle_pane`] gave up, the next redraw counts as a change and the check passes for the
/// wrong reason.
fn confirm_submit_took(target: &PaneTarget, staged: &str) -> Result<()> {
    let deadline = Instant::now() + SEND_SUBMIT_TIMEOUT;
    loop {
        thread::sleep(SEND_POLL_INTERVAL);
        if capture_pane(target, SEND_WATCH_LINES)? != staged {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "message was typed into {target} but the submit key did not take: the pane has not \
                 changed in {}s. The text is most likely still sitting unsent in the composer — \
                 check it with `niles peek`, then send again once the pane is idle.",
                SEND_SUBMIT_TIMEOUT.as_secs()
            );
        }
    }
}

/// The tmux session this process is running in.
///
/// Niles places manager and worker windows in the session the operator is already attached to.
/// Being outside tmux is therefore an error, not a cue to invent a session: a window created in
/// a session nobody is watching is indistinguishable from a worker that never started.
pub(crate) fn current_session() -> Result<SessionName> {
    if env::var_os("TMUX").is_none() {
        bail!(
            "niles must run inside tmux. Start one with `tmux new -s niles`, or attach an existing session, then rerun."
        );
    }

    let Some(name) = current_session_name()? else {
        bail!("failed to determine the current tmux session name");
    };
    SessionName::new(name)
}

pub(crate) fn ensure_window_available(session: &SessionName, window_name: &str) -> Result<()> {
    let output = output([
        "list-windows",
        "-t",
        &target::exact(session.as_str()),
        "-F",
        "#{window_name}",
    ])
    .with_context(|| format!("failed to list tmux windows in session {session}"))?;

    if !output.status.success() {
        bail!(
            "tmux list-windows failed for session {session}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    if window_name_taken(&output.stdout, window_name) {
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
    run(new_window_args(&session_target, window_name, cwd, command))
}

fn new_window_session_target(session: &SessionName) -> String {
    format!("{}:", target::exact(session.as_str()))
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

pub(crate) fn kill_window(target: &WindowTarget) -> Result<()> {
    run(["kill-window", "-t", &target.target_arg()])
}

pub(crate) fn set_window_option(target: &WindowTarget, option: &str, value: &str) -> Result<()> {
    run([
        "set-option",
        "-w",
        "-t",
        &target.target_arg(),
        option,
        value,
    ])
}

pub(crate) fn current_session_name() -> Result<Option<String>> {
    let output =
        output(["display-message", "-p", "#S"]).context("failed to query current tmux session")?;
    if !output.status.success() {
        return Ok(None);
    }

    Ok(session_name_from_stdout(&output.stdout))
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

/// Whether a window of this name exists at all, live or dead. A worker window is kept after its
/// agent exits, and it still occupies the name until the worker is closed — which is why this is
/// a different question from [`target::live_window_present`].
fn window_name_taken(stdout: &[u8], window_name: &str) -> bool {
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
    fn window_name_taken_matches_exact_window_names() {
        let output = b"niles-run\nniles-run-extra\n";

        assert!(window_name_taken(output, "niles-run"));
        assert!(!window_name_taken(output, "run"));
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
        let session = SessionName::new("niles").unwrap();
        let session_target = new_window_session_target(&session);
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
                "=niles:",
                "-n",
                "niles-auth-fix",
                "-c",
                "/tmp/workspace",
                "sh launch.sh"
            ]
        );
    }
}
