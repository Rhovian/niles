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
    /// Select a setup for workspace-local commands; an origin prefix pins the root.
    #[arg(long, global = true, env = "NILES_SETUP", value_name = "[ORIGIN:]NAME")]
    // Parse the winner only when used, so unused env cannot defeat a flag or affect services.
    pub setup: Option<String>,
    #[command(subcommand)]
    pub command: Option<CommandName>,
}

#[derive(Debug, Subcommand)]
pub enum StatusLine {
    Home,
    Projects,
    Sessions { session_name: String },
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
    /// Serve the remote API on loopback (requires --features remote).
    #[command(args_conflicts_with_subcommands = true)]
    Remote {
        #[arg(long, default_value_t = 8765)]
        port: u16,
        #[command(subcommand)]
        command: Option<RemoteCommand>,
    },
    /// Worker-facing commands.
    Agent {
        #[command(subcommand)]
        command: crate::worker::agent_status::AgentCommand,
    },
    #[command(hide = true)]
    Status {
        #[command(subcommand)]
        line: StatusLine,
    },
    /// Start the lead in a project session.
    #[command(hide = true)]
    Lead {
        #[arg(long)]
        keep_skills: bool,
    },
    /// Run the project explorer in the home session.
    #[command(hide = true)]
    Explorer,
    /// `help` is a panel, so it is not also clap's subcommand.
    #[command(hide = true, disable_help_subcommand = true)]
    Panel {
        #[command(subcommand)]
        panel: crate::projects::panels::Panel,
    },
    /// Report binary identity, tmux, and agent CLI versions
    Doctor,
    /// Run one headless turn per manifest family with Niles's launch arguments
    /// (maintainer check; makes real model calls with approval-bypass flags).
    Compat { families: Vec<String> },
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
    /// Launch a bound primary skill. Use --from to build on a completed worker's report,
    /// or --mechanical to log the reason for mechanical work in the lead session.
    ///
    /// `--wait` starts the workspace wait after spawning, so the turn does not need a
    /// separate `niles wait`.
    #[command(verbatim_doc_comment)]
    Spawn {
        /// Wait for workspace reports after spawning.
        #[arg(long)]
        wait: bool,
        /// Worker task id used for window and metadata names.
        id: String,
        #[command(flatten)]
        assignment: Box<crate::worker::Assignment>,
        /// Task label for grouping warm workers.
        #[arg(long = "task", value_name = "LABEL")]
        task_label: Option<String>,
        /// Agent id to launch; defaults to this skill's workspace manifest binding.
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
    /// not asking to exit. Keep workers warm through the send/wait loop and close only when the
    /// operator says so. Closing archives the worker's directory, so `niles report <id>` still
    /// works after.
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
    /// Switch a live worker after operator approval (same family only).
    Retier { id: String, agent: String },
    /// Append a decision to the current lead session log.
    Log {
        /// Worker id, or `-` for a session-wide decision.
        id: String,
        /// Decision and reason.
        #[arg(required = true, num_args = 1..)]
        decision: Vec<String>,
    },
    /// Print live spawned workers as JSON.
    Workers,
    /// Print usage for live lead and worker sessions as JSON.
    Usage,
    /// List effective built-in and workspace model rosters.
    Models,
    /// List bound skills, their sources, descriptions and tier primaries.
    Skills,
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
        /// Disarm this worker's check-in after successfully capturing the pane.
        #[arg(long)]
        quiet: bool,
    },
    /// Send a message to a worker tmux pane
    ///
    /// Advances the worker's wake cursor first, so a status line written before the message
    /// cannot satisfy the wait that follows it. Any actionable line stepped over is printed.
    /// Appends a reminder with this worker's report path and status-line command.
    ///
    /// `--wait` starts the workspace wait, with `niles wait`'s exit codes. Exit 11 means
    /// a newer workspace wait took over after the message was sent: do not resend.
    #[command(verbatim_doc_comment)]
    Send {
        /// Wait for workspace reports after sending.
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
    /// Wait for pending reports from every live worker in the workspace
    ///
    /// Includes workers spawned while waiting. Returns immediately when anything is pending,
    /// printing every pending report, at most one per worker: `<id> <kind>: <detail>`.
    /// Each escaped line is at most 4096 bytes. Run one wait, act on the batch, then wait again.
    /// A newer wait supersedes the running wait without consuming its reports.
    ///
    /// A worker that has not reported produces `gone:` when its window/agent exits or it stays
    /// confirmed idle for 30s. Reported workers stay quiet. Gone wakes share the report cursor.
    /// Exits 0 on wakes (reports or gone), 11 when superseded, 22 on timeout. Stdout contains only
    /// wake lines; diagnostics go to stderr. A kill after printing can replay that printed line.
    #[command(verbatim_doc_comment)]
    Wait {
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

#[derive(Debug, Subcommand)]
pub enum RemoteCommand {
    /// Remove the remote service and its Serve mounts, retaining config and push state.
    #[cfg(feature = "remote")]
    Uninstall,

    /// Check remote prerequisites without changing configuration or services.
    #[cfg(feature = "remote")]
    Doctor,

    /// Configure same-origin Tailscale Serve, authorization, and a user service.
    #[cfg(feature = "remote")]
    Setup {
        #[arg(long, default_value = "8765")]
        port: std::num::NonZeroU16,
        /// Dedicated built PWA directory containing index.html.
        #[arg(long)]
        web_dir: camino::Utf8PathBuf,
        /// Exact Tailscale login to add to the allowlist (repeatable).
        #[arg(long, required = true)]
        allow_user: Vec<String>,
        /// Initialize Web Push if absent, using this mailto: or https: contact URI.
        #[arg(long)]
        subject: Option<String>,
    },

    /// Generate a private VAPID keypair for Web Push; never overwrites existing keys.
    Init {
        /// VAPID contact URI (mailto: or https:).
        #[arg(long)]
        subject: String,
    },
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn command_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn panels_parse_with_their_range() {
        use crate::projects::panels::{Panel, Range};
        let panel = |args: &[&str]| {
            let Some(CommandName::Panel { panel }) = Cli::try_parse_from(args).unwrap().command
            else {
                panic!("not a panel");
            };
            panel
        };
        assert_eq!(panel(&["niles", "panel", "help"]), Panel::Help);
        assert_eq!(
            panel(&["niles", "panel", "telemetry", "--range", "7d"]),
            Panel::Telemetry { range: Range::Week }
        );
        assert!(Cli::try_parse_from(["niles", "panel", "telemetry"]).is_err());
        assert_eq!(
            panel(&["niles", "panel", "config"]),
            Panel::Config { project: None }
        );
        let Panel::Config {
            project: Some(name),
        } = panel(&["niles", "panel", "config", "--project", "global"])
        else {
            panic!("not a project's config");
        };
        assert_eq!(name.as_str(), "global");
        assert!(Cli::try_parse_from(["niles", "panel", "config", "--project", "a.b"]).is_err());
    }

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

        for command in [
            vec!["niles", "spawn", "--skill", "worker", "job"],
            vec!["niles", "send", "job"],
        ] {
            let conflict = [command.as_slice(), &["-m", "one", "two"]].concat();
            let error = Cli::try_parse_from(conflict).unwrap_err();
            assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);

            for (args, expected) in [
                (
                    vec!["-", "two"],
                    "'-' cannot be combined with other message words",
                ),
                (vec![], "a message is required"),
                (vec!["-m", " \t"], "a message is required"),
            ] {
                assert_eq!(
                    message(&[command.as_slice(), &args].concat())
                        .resolve()
                        .unwrap_err()
                        .to_string(),
                    expected
                );
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
        let cli = Cli::try_parse_from(["niles", "wait"]).unwrap();
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
