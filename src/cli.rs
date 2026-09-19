use camino::Utf8PathBuf;
use clap::{ArgAction, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    version = crate::build_info::CLAP_VERSION,
    about,
    infer_subcommands = true
)]
pub struct Cli {
    /// Override and persist the lead agent for bare `niles`.
    #[arg(long)]
    pub lead: Option<String>,
    #[command(subcommand)]
    pub command: Option<CommandName>,
}

#[derive(Debug, Subcommand)]
pub enum CommandName {
    /// Report binary identity, workspace schema state, and dev-mode staleness.
    Doctor,
    /// Spawn a worker agent in a tmux window.
    ///
    /// The worker's brief is the shared reporting contract plus one role fragment. Only `worker`
    /// is told to run the project's checks; `reviewer` covers correctness, idiom and economy, and
    /// `security` is the adversarial pass.
    #[command(verbatim_doc_comment)]
    Spawn {
        /// Worker task id used for window and metadata names.
        id: String,
        /// Which brief the worker gets.
        #[arg(long, value_enum, default_value_t = crate::worker::WorkerRole::Worker)]
        role: crate::worker::WorkerRole,
        /// Task label for grouping warm workers.
        #[arg(long = "task", value_name = "LABEL")]
        task_label: Option<String>,
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
        /// Worker task id followed by message.
        #[arg(required = true, num_args = 1.., trailing_var_arg = true, value_name = "ID_OR_MESSAGE")]
        target_and_message: Vec<String>,
    },
    /// Wait for the next actionable status-log wake and print it.
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
        /// Poll interval in seconds.
        #[arg(long, default_value_t = crate::wait::DEFAULT_INTERVAL_SECS)]
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
    fn lead_override_parses_without_a_session_mode_flag() {
        let cli = Cli::try_parse_from(["niles", "--lead", "claude:opus:max"]).unwrap();

        assert_eq!(cli.lead.as_deref(), Some("claude:opus:max"));
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
