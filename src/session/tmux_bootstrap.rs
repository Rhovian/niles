use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, BufRead, IsTerminal, Write},
    process::ExitStatus,
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::tmux;

const FOREGROUND_TMUX_SESSION: &str = "niles";

pub(super) fn ensure_tmux_session(tmux: Option<&OsStr>, workspace: &Utf8Path) -> Result<bool> {
    let terminal = terminal_mode_from_stdio();
    let foreground_session_exists = if tmux_session_present(tmux) || !terminal.interactive() {
        false
    } else {
        tmux::has_session(FOREGROUND_TMUX_SESSION)
    };

    match tmux_launch_action(tmux, terminal, foreground_session_exists)? {
        TmuxLaunchAction::ContinueHere => Ok(true),
        TmuxLaunchAction::StartSession { session } => {
            let status = tmux::launch_foreground_session(&session, workspace, &current_argv()?)?;
            ensure_tmux_launch_succeeded(status)?;
            Ok(false)
        }
        TmuxLaunchAction::PromptExistingSession => {
            let stdin = io::stdin();
            let mut input = stdin.lock();
            let mut output = io::stdout();
            let action =
                prompt_existing_session_action(&mut input, &mut output, FOREGROUND_TMUX_SESSION)?;
            let status = match action {
                ExistingSessionAction::AttachExisting => {
                    tmux::attach_foreground_session(FOREGROUND_TMUX_SESSION)?
                }
                ExistingSessionAction::StartNamedSession(session) => {
                    tmux::launch_foreground_session(&session, workspace, &current_argv()?)?
                }
            };
            ensure_tmux_launch_succeeded(status)?;
            Ok(false)
        }
    }
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
    AttachExisting,
    StartNamedSession(String),
}

fn prompt_existing_session_action<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    existing_session: &str,
) -> Result<ExistingSessionAction> {
    writeln!(output, "tmux session `{existing_session}` already exists.")?;
    writeln!(output, "Choose how Niles should continue:")?;
    writeln!(output, "1. Attach to `{existing_session}`.")?;
    writeln!(
        output,
        "2. Start a new tmux session with another name and run this command."
    )?;

    loop {
        write!(output, "Selection [1/2]: ")?;
        output.flush()?;

        let mut line = String::new();
        let bytes = input
            .read_line(&mut line)
            .context("failed to read tmux session selection")?;
        if bytes == 0 {
            bail!("stdin closed before tmux session selection was read");
        }

        match line.trim().to_ascii_lowercase().as_str() {
            "1" | "a" | "attach" => return Ok(ExistingSessionAction::AttachExisting),
            "2" | "n" | "new" => {
                return prompt_new_session_name(input, output, existing_session);
            }
            _ => writeln!(output, "Please answer 1 or 2.")?,
        }
    }
}

fn prompt_new_session_name<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    existing_session: &str,
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
        if session.is_empty() {
            writeln!(output, "New tmux session name cannot be empty.")?;
        } else if session == existing_session {
            writeln!(
                output,
                "New tmux session name must differ from `{existing_session}`."
            )?;
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
        let mut input = io::Cursor::new(b"attach\n".to_vec());
        let mut output = Vec::new();

        let action =
            prompt_existing_session_action(&mut input, &mut output, FOREGROUND_TMUX_SESSION)
                .unwrap();

        assert_eq!(action, ExistingSessionAction::AttachExisting);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("tmux session `niles` already exists."));
        assert!(output.contains("Selection [1/2]: "));
        assert!(!output.contains("new-session -A"));
    }

    #[test]
    fn existing_session_prompt_reads_different_new_session_name() {
        let mut input = io::Cursor::new(b"2\n\nniles\nniles-2\n".to_vec());
        let mut output = Vec::new();

        let action =
            prompt_existing_session_action(&mut input, &mut output, FOREGROUND_TMUX_SESSION)
                .unwrap();

        assert_eq!(
            action,
            ExistingSessionAction::StartNamedSession("niles-2".to_owned())
        );
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("New tmux session name cannot be empty."));
        assert!(output.contains("must differ from `niles`"));
    }
}
