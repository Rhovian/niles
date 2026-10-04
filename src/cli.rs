use anyhow::{Context, Result, bail};
use clap::{ArgAction, Args, Parser, Subcommand};
use std::{io, time::Duration};

#[derive(Debug, Parser)]
#[command(
    version = crate::build_info::CLAP_VERSION,
    about,
    infer_subcommands = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<CommandName>,
}

#[derive(Debug, Subcommand)]
pub enum StatusLine {
    Projects {
        session_name: String,
    },
    Sessions {
        session_name: String,
        window_index: u32,
    },
}

#[derive(Debug, Args)]
pub struct MessageInput {
    /// Message text. Repeat to join values with newlines.
    #[arg(
        short,
        long,
        value_name = "TEXT",
        conflicts_with = "words",
        allow_hyphen_values = true
    )]
    message: Vec<String>,
    /// Message words, or `-` to read the message from stdin.
    #[arg(value_name = "TEXT")]
    words: Vec<String>,
}

impl MessageInput {
    pub fn resolve(self) -> Result<String> {
        let message = if self.message.is_empty() {
            match self.words.as_slice() {
                [stdin] if stdin == "-" => {
                    let mut text = io::read_to_string(io::stdin())
                        .context("failed to read message from stdin")?;
                    if text.ends_with('\n') {
                        text.pop();
                    }
                    text
                }
                words if words.iter().any(|word| word == "-") => {
                    bail!("'-' cannot be combined with other message words");
                }
                words => words.join(" "),
            }
        } else {
            self.message.join("\n")
        };
        if message.trim().is_empty() {
            bail!("a message is required");
        }
        Ok(message)
    }
}

#[derive(Debug, Subcommand)]
pub enum CommandName {
    #[command(hide = true)]
    Status {
        #[command(subcommand)]
        line: StatusLine,
    },
    /// Start the lead in a project session.
    #[command(hide = true)]
    Lead,
    /// Run the project explorer in the home session.
    #[command(hide = true)]
    Explorer,
    #[command(hide = true)]
    Panel {
        #[arg(value_enum)]
        panel: crate::projects::panels::Panel,
    },
    /// Report binary identity, tmux, and agent CLI versions
    Doctor,
    /// Remove dated metadata across registered projects (preview unless --apply)
    Prune {
        /// Minimum age in days.
        #[arg(long, default_value_t = 14, value_name = "DAYS")]
        older_than: u32,
        /// Delete the listed paths.
        #[arg(long)]
        apply: bool,
    },
    /// Spawn a worker agent in a tmux window
    ///
    /// The worker's brief is the shared reporting contract plus one role fragment. Only `worker`
    /// is told to run the project's checks; `reviewer` covers correctness, idiom and economy,
    /// `security` is the adversarial pass, and `research` answers one question with cited evidence.
    ///
    /// `--wait` then blocks for this worker's first actionable line, so a single-worker turn does
    /// not need a separate `niles wait`. Leave it off when spawning a fleet and block on the group
    /// with `niles wait --task <label>` instead.
    #[command(verbatim_doc_comment)]
    Spawn {
        /// Block for this worker's first actionable wake after spawning.
        #[arg(long)]
        wait: bool,
        /// Worker task id used for window and metadata names.
        id: String,
        /// Which brief the worker gets.
        #[arg(long, value_enum, default_value_t = crate::worker::WorkerRole::Worker)]
        role: crate::worker::WorkerRole,
        /// Task label for grouping warm workers.
        #[arg(long = "task", value_name = "LABEL")]
        task_label: Option<String>,
        /// Agent id to launch; defaults to this role's workspace manifest binding.
        #[arg(short, long)]
        agent: Option<String>,
        /// Run the agent in this directory while Niles tracks it from this workspace.
        #[arg(long, value_name = "PATH", conflicts_with = "worktree")]
        tree: Option<camino::Utf8PathBuf>,
        /// Create or join a managed git worktree on this branch.
        #[arg(long, value_name = "BRANCH")]
        worktree: Option<String>,
        /// Create the worktree branch from this ref without tracking it.
        #[arg(long, value_name = "REF", requires = "worktree")]
        base: Option<String>,
        /// Check-in delay for this worker: a duration such as 1s, 90s, 5m or 1h. Defaults to
        /// this workspace's manifest `checkin`, then 5m. `0` or `off` arms none.
        #[arg(long, value_name = "DELAY")]
        checkin: Option<String>,
        #[command(flatten)]
        message: MessageInput,
    },
    /// Close spawned worker windows and archive their metadata
    ///
    /// `done:` is a handback, not a finish: a worker that reported it is waiting for a follow-up,
    /// not asking to exit. Keep workers warm through the send/wait loop and close at integration
    /// time. Closing archives the worker's directory, so `niles report <id>` still works after.
    #[command(verbatim_doc_comment)]
    Close {
        /// Worker id to close.
        #[arg(
            required_unless_present_any = ["task_label", "all"],
            conflicts_with_all = ["task_label", "all"]
        )]
        id: Option<String>,
        /// Close every live worker with this task label.
        #[arg(long = "task", value_name = "LABEL", conflicts_with = "all")]
        task_label: Option<String>,
        /// Close every live worker.
        #[arg(long, action = ArgAction::SetTrue)]
        all: bool,
    },
    /// Print live spawned workers as JSON.
    Workers,
    /// Print usage for live lead and worker sessions as JSON.
    Usage,
    /// List effective built-in and workspace model rosters.
    Models,
    /// Print a worker's durable report file.
    Report {
        /// Worker task id.
        id: String,
    },
    /// Capture the tail of a worker tmux pane.
    Peek {
        /// Worker task id.
        id: String,
        /// Number of lines to capture. Use 0 for full tmux history.
        #[arg(short, long, default_value_t = crate::worker::DEFAULT_PEEK_LINES)]
        lines: usize,
    },
    /// Send a message to a worker tmux pane
    ///
    /// Advances the worker's wake cursor first, so a status line written before the message
    /// cannot satisfy the wait that follows it. Any actionable line stepped over is printed.
    ///
    /// `--wait` then blocks for that worker's reply. With several workers in flight prefer plain
    /// `send` followed by `niles wait --task <label>`: `--wait` blocks on one worker and will not
    /// notice another finishing.
    #[command(verbatim_doc_comment)]
    Send {
        /// Block for this worker's next actionable wake after sending.
        #[arg(long)]
        wait: bool,
        /// Check-in delay for this worker's next report: a duration such as 1s, 90s, 5m or 1h.
        /// Defaults to the manifest `checkin`, then 5m. `0` or `off` arms none.
        #[arg(long, value_name = "DELAY")]
        checkin: Option<String>,
        /// Worker task id.
        id: String,
        #[command(flatten)]
        message: MessageInput,
    },
    /// Wait for the next actionable status-log wake and print it
    ///
    /// Each wait consumes one actionable line and records how far it read, so after a wake and a
    /// follow-up you run it again for the next one. Waiting on several workers returns the first
    /// line any of them produces, prefixed with its id.
    ///
    /// A worker whose tmux window has gone ends the wait rather than blocking on a log nothing
    /// can append to again — but only once its log holds no further line, so a worker that
    /// reported and then exited still hands that report over.
    ///
    /// Exits 0 on a wake, 10 when the worker closed or its window is gone, 22 on timeout.
    #[command(verbatim_doc_comment)]
    Wait {
        /// Worker ids to wait on. Pass several to wait on a fleet.
        #[arg(required_unless_present = "task", conflicts_with = "task")]
        worker: Vec<String>,
        /// Wait on every live worker carrying this task label.
        #[arg(long, required_unless_present = "worker", conflicts_with = "worker")]
        task: Option<String>,
        /// Poll interval: a duration such as 500ms, 90s, 5m or 1h. Must be greater than zero.
        #[arg(
            long,
            default_value = "2s",
            value_parser = crate::wait::parse_interval
        )]
        interval: Duration,
        /// Maximum time to wait: a duration such as 500ms, 90s, 5m or 1h. `0` checks once without
        /// waiting.
        #[arg(
            long,
            default_value = "1h",
            value_parser = crate::wait::parse_timeout
        )]
        timeout: Duration,
    },
    /// Disarm a worker's check-in, so the watcher stops nudging about it
    ///
    /// `spawn` and `send` arm one, and the watcher nudges when it comes due with no report since.
    /// Quiet a worker that is idle on purpose, so its check-ins do not keep calling the lead back.
    #[command(verbatim_doc_comment)]
    Quiet {
        /// Worker task id.
        id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_niles_has_no_subcommand() {
        let cli = Cli::try_parse_from(["niles"]).unwrap();

        assert!(cli.command.is_none());
    }

    #[test]
    fn message_input_parses_options_and_rejects_invalid_sources() {
        fn message(args: &[&str]) -> MessageInput {
            let command = Cli::try_parse_from(args).unwrap().command;
            if let Some(CommandName::Spawn { message, .. }) = command {
                return message;
            }
            if let Some(CommandName::Send { message, .. }) = command {
                return message;
            }
            panic!("expected spawn or send command");
        }

        for command in ["spawn", "send"] {
            let conflict = ["niles", command, "job", "-m", "one", "two"];
            let error = Cli::try_parse_from(conflict).unwrap_err();
            assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);

            for (args, expected) in [
                (
                    vec!["niles", command, "job", "-", "two"],
                    "'-' cannot be combined with other message words",
                ),
                (vec!["niles", command, "job"], "a message is required"),
                (
                    vec!["niles", command, "job", "-m", " \t"],
                    "a message is required",
                ),
            ] {
                assert_eq!(message(&args).resolve().unwrap_err().to_string(), expected);
            }
        }

        let cli =
            Cli::try_parse_from(["niles", "send", "job", "-m", "- item", "-m", "b", "--wait"])
                .unwrap();
        let Some(CommandName::Send { wait, message, .. }) = cli.command else {
            panic!("expected send command");
        };
        assert!(wait);
        assert_eq!(message.resolve().unwrap(), "- item\nb");
    }

    #[test]
    fn wait_default_spellings_parse_to_the_runtime_defaults() {
        let cli = Cli::try_parse_from(["niles", "wait", "worker"]).unwrap();
        let Some(CommandName::Wait {
            interval, timeout, ..
        }) = cli.command
        else {
            panic!("expected wait command");
        };

        assert_eq!(interval, crate::wait::DEFAULT_INTERVAL);
        assert_eq!(timeout, crate::wait::DEFAULT_TIMEOUT);
    }
}
