use crate::commands::{Approve, Close, Message, Peek, SendArgs, SpawnArgs, Target, Wait};
use crate::harness::LaunchArgs;
use crate::projects::onboard::unattended::Options;
use crate::projects::panels::Panel;
use crate::skills::SkillsCommand;
use crate::update::Channel;
use crate::worker::Spawned;
use crate::worker::agent_status::AgentCommand;
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use std::io;

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
    pub fn resolve(self) -> Result<Message> {
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
        message.try_into()
    }
}

#[derive(Debug, Subcommand)]
pub enum CommandName {
    /// Serve the configured remote listeners (requires --features remote).
    #[command(args_conflicts_with_subcommands = true)]
    Remote {
        /// Port for the tailnet relay API. Set remote.mcp.port for MCP.
        #[arg(long, default_value_t = 8765)]
        port: u16,
        #[command(subcommand)]
        command: Option<RemoteCommand>,
    },
    /// Agent-facing commands.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    #[command(hide = true)]
    Status {
        #[command(subcommand)]
        line: StatusLine,
    },
    /// Launch the start session in a project.
    #[command(hide = true)]
    Start {
        #[command(flatten)]
        launch: LaunchArgs,
        #[arg(long)]
        keep_skills: bool,
        /// Launch the entry session on this listed tier agent instead of the manifest's
        /// `entry_agent`.
        #[arg(short, long)]
        agent: Option<String>,
    },
    /// Run the project explorer in the home session.
    #[command(hide = true)]
    Explorer,
    /// Run an ACP adapter in this window.
    #[command(hide = true)]
    Acp {
        dir: camino::Utf8PathBuf,
        prompt: String,
    },
    /// Request cancellation of a headless worker turn.
    Cancel(Target),
    /// Answer a headless permission; omit the option to cancel the request.
    Approve(Approve),
    /// Resume a headless worker conversation.
    Resume { id: String },
    /// `help` is a panel, so it is not also clap's subcommand.
    #[command(hide = true, disable_help_subcommand = true)]
    Panel {
        #[command(subcommand)]
        panel: Panel,
    },
    /// Report binary identity, tmux, and agent CLI versions
    Doctor,
    /// Configure this project without the interactive wizard (does not overwrite).
    Setup(Box<Options>),
    /// Update a shell installation, or show the package-manager command
    Update {
        /// Release channel (nightly includes prereleases).
        #[arg(long, value_enum, default_value = "stable")]
        channel: Channel,
    },
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
    /// Launch an installed primary skill.
    #[command(verbatim_doc_comment)]
    Spawn {
        #[command(flatten)]
        args: Box<SpawnArgs>,
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
    Close(Close),
    /// Append a decision to the current start session log.
    Log {
        /// Worker id, or `-` for a session-wide decision.
        id: String,
        /// Decision and reason.
        #[arg(required = true, num_args = 1..)]
        decision: Vec<String>,
    },
    /// Print live spawned workers as JSON.
    Workers,
    /// Print usage for live start and worker sessions as JSON.
    Usage,
    /// List models and efforts from installed agent CLIs.
    Models,
    /// List installed skills, sources and descriptions.
    Skills {
        #[command(subcommand)]
        command: Option<SkillsCommand>,
    },
    /// Print a worker's durable report file.
    Report(Target),
    /// Capture the tail of a worker tmux pane.
    Peek(Peek),
    /// Send a message to a worker tmux pane
    ///
    /// Advances the worker's wake cursor first, so a status line written before the message
    /// cannot satisfy the wait that follows it. Any actionable line stepped over is printed.
    /// Appends a reminder with this worker's report path and status-line command.
    #[command(verbatim_doc_comment)]
    Send {
        #[command(flatten)]
        args: SendArgs,
        #[command(flatten)]
        message: MessageInput,
    },
    /// Wait for pending reports from every live worker in the workspace
    ///
    /// Includes workers spawned while waiting. Returns immediately when anything is pending,
    /// printing every pending report, at most one per worker: `<id> <kind>: <detail>`.
    /// Each escaped wake line is at most 4096 bytes. Pass the final ack token with --ack
    /// on the next wait after receiving the batch. Wakes can replay until acknowledged.
    /// A newer wait supersedes the running wait without consuming its reports.
    ///
    /// A worker that has not reported produces `gone:` when its window/agent exits or it stays
    /// confirmed idle for 30s. Reported workers stay quiet. Gone wakes share the report cursor.
    /// Exits 0 on wakes (reports or gone), 11 when superseded, 22 on timeout. Stdout contains only
    /// the wake batch and its final acknowledgement line. Diagnostics go to stderr.
    #[command(verbatim_doc_comment)]
    Wait(Wait),
    /// Disarm a worker's check-in, so the watcher stops nudging about it
    ///
    /// `spawn` and `send` arm one, and the watcher nudges when it comes due with no report since.
    /// Quiet a worker that is idle on purpose, so its check-ins do not keep waking the start session.
    #[command(verbatim_doc_comment)]
    Quiet {
        /// Worker task id.
        id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum RemoteCommand {
    /// Generate a private VAPID keypair for Web Push; never overwrites existing keys.
    Init {
        /// VAPID contact URI (mailto: or https:).
        #[arg(long)]
        subject: String,
    },
}

pub(crate) fn send_hints(out: &mut dyn io::Write, id: &str) -> Result<()> {
    writeln!(out, "wait: niles wait")?;
    writeln!(out, "peek: niles peek {id}")?;
    Ok(())
}

pub(crate) fn spawn_hints(out: &mut dyn io::Write, worker: Spawned) -> Result<()> {
    let id = &worker.id;
    send_hints(out, id)?;
    writeln!(out, "report: niles report {id}")?;
    writeln!(out, "send: niles send {id} <message>")?;
    writeln!(out, "close: niles close {id}")?;
    if let Some(label) = worker.task_label {
        writeln!(out, "close_task: niles close --task {label}")?;
    }
    writeln!(out, "workers: niles workers")?;
    if worker.checkin_armed {
        writeln!(out, "quiet: niles quiet {id}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, error::ErrorKind};

    #[test]
    fn command_definition_is_consistent() {
        Cli::command().debug_assert();
        assert!(Cli::try_parse_from(["niles"]).unwrap().command.is_none());
        assert!(Cli::try_parse_from(["niles", "mcp"]).is_err());
    }

    #[test]
    fn start_replaces_the_lead_subcommand() {
        for (args, expected) in [
            (vec!["niles", "start"], false),
            (vec!["niles", "start", "--keep-skills"], true),
        ] {
            assert!(matches!(Cli::try_parse_from(args).unwrap().command,
                Some(CommandName::Start { keep_skills, .. }) if keep_skills == expected));
        }
        assert_eq!(
            Cli::try_parse_from(["niles", "lead"]).unwrap_err().kind(),
            ErrorKind::InvalidSubcommand
        );
    }

    #[test]
    fn panels_parse_with_their_range() {
        use crate::projects::panels::{
            Panel::{Config, Help, Telemetry},
            Range::Week,
        };
        let parse = |args: &str| {
            let argv = ["niles", "panel"]
                .into_iter()
                .chain(args.split_whitespace());
            Cli::try_parse_from(argv)
        };
        for (args, expected) in [
            ("help", Help),
            ("telemetry --range 7d", Telemetry { range: Week }),
            ("config", Config { project: None }),
        ] {
            let actual = parse(args).unwrap().command;
            assert!(matches!(actual, Some(CommandName::Panel { panel }) if panel == expected));
        }
        assert!(matches!(parse("config --project global").unwrap().command,
            Some(CommandName::Panel { panel: Config { project: Some(name) } })
            if name.as_str() == "global"));
        for args in ["telemetry", "config --project a.b"] {
            assert!(parse(args).is_err(), "{args}");
        }
    }

    #[test]
    fn message_input_parses_options_and_rejects_invalid_sources() {
        fn message(args: &[&str]) -> MessageInput {
            match Cli::try_parse_from(args).unwrap().command {
                Some(CommandName::Spawn { message, .. } | CommandName::Send { message, .. }) => {
                    Some(message)
                }
                _ => None,
            }
            .expect("spawn or send command")
        }

        for command in [
            vec!["niles", "spawn", "--skill", "worker", "job"],
            vec!["niles", "send", "job"],
        ] {
            let conflict = [command.as_slice(), &["-m", "one", "two"]].concat();
            let error = Cli::try_parse_from(conflict).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
            let combined_wait = [command.as_slice(), &["--wait", "task"]].concat();
            assert_eq!(
                Cli::try_parse_from(combined_wait).unwrap_err().kind(),
                ErrorKind::UnknownArgument
            );

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

        let cli = Cli::try_parse_from([
            "niles",
            "send",
            "job",
            "-m",
            "- item",
            "-m",
            "b",
            "--checkin",
            "off",
        ])
        .unwrap();
        let message = match cli.command {
            Some(CommandName::Send { message, .. }) => Some(message),
            _ => None,
        }
        .expect("send command");
        assert_eq!(
            message.resolve().unwrap(),
            "- item\nb".to_owned().try_into().unwrap()
        );
    }

    #[test]
    fn wait_default_spellings_parse_to_the_runtime_defaults() {
        let cli = Cli::try_parse_from(["niles", "wait"]).unwrap();
        assert!(matches!(cli.command, Some(CommandName::Wait(args)) if args == Wait::default()));
    }
}
