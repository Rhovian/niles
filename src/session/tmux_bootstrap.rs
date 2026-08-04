use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, BufRead, IsTerminal, Write},
    process::ExitStatus,
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::tmux;

use super::manager_window::MANAGER_WINDOW_NAME;

const FOREGROUND_TMUX_SESSION: &str = "niles";

pub(super) fn ensure_tmux_session(
    tmux: Option<&OsStr>,
    workspace: &Utf8Path,
    requested: Option<&str>,
) -> Result<bool> {
    let terminal = terminal_mode_from_stdio();

    if let Some(requested) = requested {
        return ensure_requested_session(tmux, workspace, requested);
    }

    // Attachable sessions are the Niles ones plus `niles` itself, which may
    // exist without a manager window and would otherwise collide with a fresh
    // start under that name.
    let candidates = if tmux_session_present(tmux) || !terminal.interactive() {
        Vec::new()
    } else {
        session_candidates(
            FOREGROUND_TMUX_SESSION,
            tmux::has_session(FOREGROUND_TMUX_SESSION),
            &live_manager_sessions()?,
        )
    };

    match tmux_launch_action(tmux, terminal, !candidates.is_empty())? {
        TmuxLaunchAction::ContinueHere => Ok(true),
        TmuxLaunchAction::StartSession { session } => {
            let session = tmux::SessionName::new(session)?;
            let status = tmux::launch_foreground_session(&session, workspace, &current_argv()?)?;
            ensure_tmux_launch_succeeded(status)?;
            Ok(false)
        }
        TmuxLaunchAction::PromptExistingSession => {
            let stdin = io::stdin();
            let mut input = stdin.lock();
            let mut output = io::stdout();
            let action = prompt_existing_session_action(
                &mut input,
                &mut output,
                &candidates,
                tmux::has_session,
            )?;
            let status = match action {
                ExistingSessionAction::Attach(session) => {
                    tmux::attach_foreground_session(&tmux::SessionName::requested(session)?)?
                }
                ExistingSessionAction::StartNamedSession(session) => {
                    let session = tmux::SessionName::requested(session)?;
                    tmux::launch_foreground_session(&session, workspace, &current_argv()?)?
                }
            };
            ensure_tmux_launch_succeeded(status)?;
            Ok(false)
        }
    }
}

/// `--session NAME`: attach when it is live, otherwise start it. No prompt.
fn ensure_requested_session(
    tmux: Option<&OsStr>,
    workspace: &Utf8Path,
    requested: &str,
) -> Result<bool> {
    let session = tmux::SessionName::requested(requested)?;

    // Starting a session re-runs this same argv inside it, so the second pass
    // arrives with TMUX set. Continuing there is the tail of that one hop; any
    // other session means the flag would silently do nothing.
    if tmux_session_present(tmux) {
        return match tmux::current_session_name()? {
            Some(current) if current == session.as_str() => Ok(true),
            Some(current) => bail!(
                "already inside tmux session `{current}`, so `--session {session}` cannot be honored; detach first, or run `tmux switch-client -t '={session}'`"
            ),
            None => bail!("failed to determine the current tmux session name"),
        };
    }

    let status = if tmux::has_session(session.as_str()) {
        tmux::attach_foreground_session(&session)?
    } else {
        tmux::launch_foreground_session(&session, workspace, &current_argv()?)?
    };
    ensure_tmux_launch_succeeded(status)?;
    Ok(false)
}

/// Live Niles sessions that can actually be targeted, with the rest reported
/// rather than dropped — a healthy session vanishing silently is its own bug.
fn live_manager_sessions() -> Result<Vec<String>> {
    let mut addressable = Vec::new();
    for session in tmux::live_manager_sessions(MANAGER_WINDOW_NAME)? {
        match tmux::unaddressable_reason(&session) {
            None => addressable.push(session),
            Some(reason) => {
                eprintln!("note: skipping tmux session `{session}`: {reason}");
            }
        }
    }
    Ok(addressable)
}

fn ensure_tmux_launch_succeeded(status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        bail!("tmux foreground launch exited with {status}")
    }
}

fn current_argv() -> Result<Vec<OsString>> {
    let argv = env::args_os().collect::<Vec<_>>();
    if argv.is_empty() {
        bail!("failed to determine original argv for tmux foreground launch");
    }
    Ok(argv)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerminalMode {
    stdin: bool,
    stdout: bool,
}

impl TerminalMode {
    fn interactive(self) -> bool {
        self.stdin && self.stdout
    }
}

fn terminal_mode_from_stdio() -> TerminalMode {
    TerminalMode {
        stdin: io::stdin().is_terminal(),
        stdout: io::stdout().is_terminal(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TmuxLaunchAction {
    ContinueHere,
    StartSession { session: &'static str },
    PromptExistingSession,
}

fn tmux_launch_action(
    tmux: Option<&OsStr>,
    terminal: TerminalMode,
    foreground_session_exists: bool,
) -> Result<TmuxLaunchAction> {
    if tmux_session_present(tmux) {
        return Ok(TmuxLaunchAction::ContinueHere);
    }

    if !terminal.interactive() {
        bail!(
            "Niles must launch the foreground manager from an attached tmux session. Because stdin and stdout are not both TTYs, Niles will not auto-start tmux; start or attach tmux interactively and run `niles` again."
        );
    }

    if foreground_session_exists {
        Ok(TmuxLaunchAction::PromptExistingSession)
    } else {
        Ok(TmuxLaunchAction::StartSession {
            session: FOREGROUND_TMUX_SESSION,
        })
    }
}

fn tmux_session_present(tmux: Option<&OsStr>) -> bool {
    tmux.and_then(OsStr::to_str)
        .is_some_and(|value| !value.trim().is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExistingSessionAction {
    Attach(String),
    StartNamedSession(String),
}

/// Every session bare `niles` could attach to: the live Niles sessions, plus
/// `default_session` when it exists without a manager window — starting a new
/// session under that name would collide.
fn session_candidates(default_session: &str, default_exists: bool, live: &[String]) -> Vec<String> {
    let mut candidates = Vec::new();
    if default_exists {
        candidates.push(default_session.to_owned());
    }
    candidates.extend(
        live.iter()
            .filter(|session| *session != default_session)
            .cloned(),
    );
    candidates
}

fn prompt_existing_session_action<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    candidates: &[String],
    session_exists: impl Fn(&str) -> bool,
) -> Result<ExistingSessionAction> {
    let new_session_choice = candidates.len() + 1;

    writeln!(output, "Live Niles sessions:")?;
    for (index, session) in candidates.iter().enumerate() {
        writeln!(output, "  {}. {session}", index + 1)?;
    }
    writeln!(
        output,
        "  {new_session_choice}. Start a new session with another name."
    )?;

    loop {
        write!(output, "Selection [1-{new_session_choice}]: ")?;
        output.flush()?;

        let mut line = String::new();
        let bytes = input
            .read_line(&mut line)
            .context("failed to read tmux session selection")?;
        if bytes == 0 {
            bail!("stdin closed before tmux session selection was read");
        }

        match line.trim().parse::<usize>() {
            Ok(choice) if choice == new_session_choice => {
                return prompt_new_session_name(input, output, candidates, session_exists);
            }
            Ok(choice) if (1..=candidates.len()).contains(&choice) => {
                return Ok(ExistingSessionAction::Attach(
                    candidates[choice - 1].clone(),
                ));
            }
            _ => writeln!(output, "Please answer 1 to {new_session_choice}.")?,
        }
    }
}

fn prompt_new_session_name<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    taken: &[String],
    session_exists: impl Fn(&str) -> bool,
) -> Result<ExistingSessionAction> {
    loop {
        write!(output, "New tmux session name: ")?;
        output.flush()?;

        let mut line = String::new();
        let bytes = input
            .read_line(&mut line)
            .context("failed to read new tmux session name")?;
        if bytes == 0 {
            bail!("stdin closed before new tmux session name was read");
        }

        let session = line.trim();
        // `taken` only covers sessions we listed; tmux owns the real answer,
        // so an unlisted collision still fails loudly at launch.
        if taken.iter().any(|name| name == session) || session_exists(session) {
            writeln!(output, "tmux session `{session}` already exists.")?;
        } else if let Some(reason) = tmux::unaddressable_reason(session) {
            writeln!(output, "Invalid tmux session name: {reason}.")?;
        } else {
            return Ok(ExistingSessionAction::StartNamedSession(session.to_owned()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_session_present_accepts_nonempty_env() {
        assert!(tmux_session_present(Some(OsStr::new(
            "/tmp/tmux-501/default,1,0"
        ))));
    }

    #[test]
    fn tmux_session_present_rejects_missing_or_empty_env() {
        assert!(!tmux_session_present(None));
        assert!(!tmux_session_present(Some(OsStr::new(""))));
        assert!(!tmux_session_present(Some(OsStr::new("   "))));
    }

    #[test]
    fn tmux_launch_action_continues_inside_tmux_even_without_tty() {
        let action = tmux_launch_action(
            Some(OsStr::new("/tmp/tmux-501/default,1,0")),
            TerminalMode {
                stdin: false,
                stdout: false,
            },
            true,
        )
        .unwrap();

        assert_eq!(action, TmuxLaunchAction::ContinueHere);
    }

    #[test]
    fn tmux_launch_action_starts_session_for_interactive_no_tmux() {
        let action = tmux_launch_action(
            None,
            TerminalMode {
                stdin: true,
                stdout: true,
            },
            false,
        )
        .unwrap();

        assert_eq!(
            action,
            TmuxLaunchAction::StartSession {
                session: FOREGROUND_TMUX_SESSION,
            }
        );
    }

    #[test]
    fn tmux_launch_action_prompts_for_interactive_existing_session() {
        let action = tmux_launch_action(
            None,
            TerminalMode {
                stdin: true,
                stdout: true,
            },
            true,
        )
        .unwrap();

        assert_eq!(action, TmuxLaunchAction::PromptExistingSession);
    }

    #[test]
    fn tmux_launch_action_errors_for_non_tty_no_tmux() {
        let err = tmux_launch_action(
            None,
            TerminalMode {
                stdin: true,
                stdout: false,
            },
            true,
        )
        .unwrap_err();

        assert!(err.to_string().contains("not both TTYs"));
        assert!(err.to_string().contains("will not auto-start tmux"));
    }

    #[test]
    fn existing_session_prompt_can_attach_existing_session() {
        let mut input = io::Cursor::new(b"1\n".to_vec());
        let mut output = Vec::new();

        let action = prompt_existing_session_action(
            &mut input,
            &mut output,
            &[FOREGROUND_TMUX_SESSION.to_owned()],
            |_| false,
        )
        .unwrap();

        assert_eq!(
            action,
            ExistingSessionAction::Attach(FOREGROUND_TMUX_SESSION.to_owned())
        );
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Live Niles sessions:"));
        assert!(output.contains("  1. niles"));
        assert!(output.contains("Selection [1-2]: "));
    }

    #[test]
    fn a_live_session_under_another_name_is_still_offered() {
        // The whole point of the feature: without `niles` itself running, a
        // session named `aquila` used to be unreachable.
        let candidates = session_candidates(FOREGROUND_TMUX_SESSION, false, &["aquila".to_owned()]);

        assert_eq!(candidates, ["aquila"]);
    }

    #[test]
    fn the_default_session_is_offered_even_without_a_manager_window() {
        // Otherwise niles tries to start `niles` and tmux rejects the duplicate.
        let candidates = session_candidates(FOREGROUND_TMUX_SESSION, true, &[]);

        assert_eq!(candidates, ["niles"]);
    }

    #[test]
    fn nothing_running_means_no_prompt_at_all() {
        assert!(session_candidates(FOREGROUND_TMUX_SESSION, false, &[]).is_empty());
    }

    #[test]
    fn the_default_session_is_never_listed_twice() {
        let live = ["niles".to_owned(), "aquila".to_owned()];

        assert_eq!(
            session_candidates(FOREGROUND_TMUX_SESSION, true, &live),
            ["niles", "aquila"]
        );
    }

    #[test]
    fn existing_session_prompt_lists_every_live_niles_session() {
        let mut input = io::Cursor::new(b"3\n".to_vec());
        let mut output = Vec::new();
        let live = ["niles".to_owned(), "aquila".to_owned(), "orion".to_owned()];

        let action =
            prompt_existing_session_action(&mut input, &mut output, &live, |_| false).unwrap();

        // The default session is listed once, not duplicated by the live scan.
        assert_eq!(action, ExistingSessionAction::Attach("orion".to_owned()));
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("  1. niles"));
        assert!(output.contains("  2. aquila"));
        assert!(output.contains("  3. orion"));
        assert!(output.contains("  4. Start a new session with another name."));
    }

    #[test]
    fn existing_session_prompt_reprompts_on_out_of_range_and_junk() {
        let mut input = io::Cursor::new(b"0\n9\nabc\n1\n".to_vec());
        let mut output = Vec::new();

        let action = prompt_existing_session_action(
            &mut input,
            &mut output,
            &[FOREGROUND_TMUX_SESSION.to_owned()],
            |_| false,
        )
        .unwrap();

        assert_eq!(
            action,
            ExistingSessionAction::Attach(FOREGROUND_TMUX_SESSION.to_owned())
        );
        assert_eq!(
            String::from_utf8(output)
                .unwrap()
                .matches("Please answer")
                .count(),
            3
        );
    }

    #[test]
    fn new_session_name_prompt_rejects_names_tmux_cannot_target() {
        let mut input = io::Cursor::new(b"2\nmy.proj\nniles-2\n".to_vec());
        let mut output = Vec::new();

        let action = prompt_existing_session_action(
            &mut input,
            &mut output,
            &[FOREGROUND_TMUX_SESSION.to_owned()],
            |_| false,
        )
        .unwrap();

        assert_eq!(
            action,
            ExistingSessionAction::StartNamedSession("niles-2".to_owned())
        );
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("`:` and `.` as target separators")
        );
    }

    #[test]
    fn new_session_name_prompt_rejects_a_session_that_is_live_but_unlisted() {
        let mut input = io::Cursor::new(b"2\nscratch\nniles-2\n".to_vec());
        let mut output = Vec::new();

        let action = prompt_existing_session_action(
            &mut input,
            &mut output,
            &[FOREGROUND_TMUX_SESSION.to_owned()],
            |name| name == "scratch",
        )
        .unwrap();

        assert_eq!(
            action,
            ExistingSessionAction::StartNamedSession("niles-2".to_owned())
        );
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("tmux session `scratch` already exists.")
        );
    }

    #[test]
    fn existing_session_prompt_reads_different_new_session_name() {
        let mut input = io::Cursor::new(b"2\n\nniles\nniles-2\n".to_vec());
        let mut output = Vec::new();

        let action = prompt_existing_session_action(
            &mut input,
            &mut output,
            &[FOREGROUND_TMUX_SESSION.to_owned()],
            |_| false,
        )
        .unwrap();

        assert_eq!(
            action,
            ExistingSessionAction::StartNamedSession("niles-2".to_owned())
        );
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("name cannot be empty"));
        assert!(output.contains("tmux session `niles` already exists."));
    }
}
