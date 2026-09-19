use camino::Utf8PathBuf;
use clap::{ArgAction, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    version = crate::build_info::CLAP_VERSION,
    about,
    infer_subcommands = true
)]
pub struct Cli {
    /// Override and persist the manager agent for bare `niles`.
    #[arg(long)]
    pub manager: Option<String>,
    #[command(subcommand)]
    pub command: Option<CommandName>,
}

#[derive(Debug, Subcommand)]
pub enum CommandName {
    /// Probe configured agent CLIs and write local capability manifests.
    #[command(alias = "scan")]
    Analyze {
        /// Agent id to probe. Defaults to codex and claude.
        #[arg(short, long)]
        agent: Option<String>,
    },
    /// Report binary identity, workspace schema state, and dev-mode staleness.
    Doctor,
    /// Spawn a worker agent in a tmux window.
    Spawn {
        /// Proceed even when a built-in agent CLI is below the pinned version range.
        #[arg(long, env = "NILES_ALLOW_CLI_MISMATCH")]
        allow_cli_mismatch: bool,
        /// Worker task id used for window and metadata names.
        id: String,
        /// Task label for grouping warm workers.
        #[arg(long = "task", value_name = "LABEL")]
        task_label: Option<String>,
        /// Current workspace for the worker. Other paths are rejected.
        #[arg(long, default_value = ".")]
        project: Utf8PathBuf,
        /// Agent id to launch.
        #[arg(short, long, default_value = "codex")]
        agent: String,
        /// Existing brief file to pass to the worker.
        #[arg(long)]
        brief: Option<Utf8PathBuf>,
        /// Task text used to create a brief when --brief is omitted.
        #[arg(num_args = 0.., trailing_var_arg = true)]
        task: Vec<String>,
    },
    /// Close spawned worker windows and archive their metadata.
    #[command(name = "worker-close")]
    WorkerClose {
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
    /// List live spawned workers.
    Workers,
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
    /// Send a message to a worker tmux pane.
    Send {
        /// Worker task id followed by message.
        #[arg(required = true, num_args = 1.., trailing_var_arg = true, value_name = "ID_OR_MESSAGE")]
        target_and_message: Vec<String>,
    },
    /// Wait for the next actionable status-log wake and print it.
    Wait {
        /// Worker id to wait on; repeatable to wait on a fleet.
        #[arg(
            long,
            action = ArgAction::Append,
            required_unless_present = "task",
            conflicts_with = "task"
        )]
        worker: Vec<String>,
        /// Wait on every live worker carrying this task label.
        #[arg(long, required_unless_present = "worker", conflicts_with = "worker")]
        task: Option<String>,
        /// Poll interval in seconds.
        #[arg(long, default_value_t = 2.0)]
        interval: f64,
        /// Maximum seconds to wait before exiting non-zero. Defaults to 3600 seconds.
        #[arg(long)]
        timeout: Option<f64>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_niles_runs_the_manager_with_no_subcommand() {
        let cli = Cli::try_parse_from(["niles"]).unwrap();

        assert!(cli.command.is_none());
    }

    #[test]
    fn manager_override_parses_without_a_session_mode_flag() {
        let cli = Cli::try_parse_from(["niles", "--manager", "claude:opus:max"]).unwrap();

        assert_eq!(cli.manager.as_deref(), Some("claude:opus:max"));
        assert!(cli.command.is_none());
    }

    #[test]
    fn retired_session_flags_are_rejected() {
        for retired in [
            vec!["niles", "--session", "niles"],
            vec!["niles", "--detached"],
            vec!["niles", "-d"],
        ] {
            assert!(
                Cli::try_parse_from(&retired).is_err(),
                "{retired:?} should no longer parse"
            );
        }
    }

    #[test]
    fn subcommands_still_parse() {
        let cli = Cli::try_parse_from(["niles", "workers"]).unwrap();

        assert!(cli.command.is_some());
    }
}
