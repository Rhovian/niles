use camino::Utf8PathBuf;
use clap::{ArgAction, Parser, Subcommand};

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

/// Pulls a leading `--wait` out of a trailing var-arg list, reporting whether it was there.
///
/// `spawn`'s task text and `send`'s message are both trailing var-args, so clap hands `--wait`
/// over as ordinary text whenever it is written after the positional that starts them — which is
/// exactly where it gets typed. Without this it would be passed through to the agent.
pub(crate) fn take_leading_wait(args: &mut Vec<String>) -> bool {
    let present = args.first().is_some_and(|first| first == WAIT_FLAG);
    if present {
        args.remove(0);
    }
    present
}

const WAIT_FLAG: &str = "--wait";
const CHECKIN_FLAG: &str = "--checkin";

/// Pulls a leading `--checkin <delay>` pair out of a trailing var-arg list.
///
/// A `--checkin` with nothing after it is a typo rather than a message, so the flag is consumed
/// and no delay is reported: either way the text must not reach the worker.
pub(crate) fn take_leading_checkin(args: &mut Vec<String>) -> Option<String> {
    let present = args.first().is_some_and(|first| first == CHECKIN_FLAG);
    if !present {
        return None;
    }
    args.remove(0);
    if args.is_empty() {
        return None;
    }
    Some(args.remove(0))
}

/// Pulls both dispatch flags out of a trailing var-arg list, in whatever order they were written.
///
/// One flag alone is easy — write it before the trailing text and clap parses it. Two of them next
/// to each other after the worker id arrive as text, and pulling only the first would hand the
/// worker the flag that was left behind.
pub(crate) fn take_leading_dispatch_flags(args: &mut Vec<String>) -> (bool, Option<String>) {
    let mut wait = false;
    let mut checkin = None;
    loop {
        if take_leading_wait(args) {
            wait = true;
            continue;
        }
        match take_leading_checkin(args) {
            Some(delay) => checkin = Some(delay),
            None => break,
        }
    }
    (wait, checkin)
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
        /// Existing brief file to pass to the worker.
        #[arg(long)]
        brief: Option<Utf8PathBuf>,
        /// Check-in delay for this worker: `90s`, `5m`, `1h`, or bare minutes. `0`/`off` arms none.
        #[arg(long, value_name = "DELAY")]
        checkin: Option<String>,
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
        /// Check-in delay for this worker's next report: `90s`, `5m`, `1h`, bare minutes, `0`/`off`.
        #[arg(long, value_name = "DELAY")]
        checkin: Option<String>,
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
    /// Disarm a worker's check-in, so the watcher stops nudging about it.
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
    fn bare_niles_runs_the_manager_with_no_subcommand() {
        let cli = Cli::try_parse_from(["niles"]).unwrap();

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

    /// The lead writes these after the worker id, where the trailing var-arg positional hands them
    /// over as text. In whatever order they come, the message that reaches the worker is the
    /// message alone.
    #[test]
    fn trailing_dispatch_flags_are_taken_out_of_the_message() {
        fn taken(written: &[&str]) -> (Vec<String>, bool, Option<String>) {
            let mut args = written
                .iter()
                .map(|arg| (*arg).to_owned())
                .collect::<Vec<_>>();
            let (wait, checkin) = take_leading_dispatch_flags(&mut args);
            (args, wait, checkin)
        }
        fn left(rest: &[&str]) -> Vec<String> {
            rest.iter().map(|arg| (*arg).to_owned()).collect()
        }

        let both = |delay: &str| (left(&["carry on"]), true, Some(delay.to_owned()));
        assert_eq!(
            taken(&["--wait", "--checkin", "5m", "carry on"]),
            both("5m")
        );
        assert_eq!(
            taken(&["--checkin", "90s", "--wait", "carry on"]),
            both("90s")
        );

        assert_eq!(
            taken(&["--checkin", "1h"]),
            (left(&[]), false, Some("1h".to_owned()))
        );
        assert_eq!(taken(&["--wait"]), (left(&[]), true, None));
        // A `--checkin` with nothing after it is a typo: the flag goes, no delay is invented, and
        // the text must not reach the worker either way.
        assert_eq!(taken(&["--checkin"]), (left(&[]), false, None));
        // Not leading: this is the message, and the flags in it are the worker's business.
        assert_eq!(
            taken(&["carry on", "--wait"]),
            (left(&["carry on", "--wait"]), false, None)
        );
    }
}
