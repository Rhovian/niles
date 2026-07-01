use camino::Utf8PathBuf;
use clap::{ArgAction, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(version, about, infer_subcommands = true)]
pub struct Cli {
    /// Agent id to launch for bare `niles`.
    #[arg(long, default_value = "claude")]
    pub supervisor: String,
    /// Initial goal to include in the foreground supervisor brief.
    #[arg(long)]
    pub goal: Option<String>,
    #[command(subcommand)]
    pub command: Option<CommandName>,
}

#[derive(Debug, Subcommand)]
pub enum CommandName {
    /// Start a one-off agent task without writing YAML.
    #[command(alias = "a")]
    Ask {
        /// Agent id to hand the task to.
        #[arg(short, long, default_value = "codex")]
        agent: String,
        /// Prompt to send to the agent.
        #[arg(required = true, action = ArgAction::Append, trailing_var_arg = true)]
        prompt: Vec<String>,
    },
    /// Probe configured agent CLIs and write local capability manifests.
    #[command(alias = "doctor", alias = "scan")]
    Analyze {
        /// Agent id to probe. Defaults to codex and claude.
        #[arg(short, long)]
        agent: Option<String>,
    },
    /// Prepare a new supervisor-driven run from a task spec.
    #[command(alias = "r")]
    Run {
        /// Proceed even when a built-in agent CLI is below the pinned version range.
        #[arg(long, env = "NILES_ALLOW_CLI_MISMATCH")]
        allow_cli_mismatch: bool,
        /// YAML task specification.
        task: Utf8PathBuf,
    },
    /// Launch a single run step in its own tmux window.
    Step {
        /// Run id or "latest".
        #[arg(default_value = "latest")]
        run: String,
        /// Step number to launch. Defaults to the first pending step.
        #[arg(short, long)]
        index: Option<usize>,
    },
    /// Append a step to an existing run (for supervisor-driven review loops).
    #[command(name = "step-add")]
    StepAdd {
        /// Run id or "latest".
        #[arg(default_value = "latest")]
        run: String,
        /// Agent id for an agent step.
        #[arg(short, long, conflicts_with = "command")]
        agent: Option<String>,
        /// Named command for a command step.
        #[arg(long, conflicts_with = "agent")]
        command: Option<String>,
        /// Role label for the step.
        #[arg(short, long)]
        role: Option<String>,
        /// Task text (required for an agent step).
        #[arg(num_args = 0.., trailing_var_arg = true)]
        task: Vec<String>,
    },
    /// Mark a step complete and close its interactive tmux window.
    #[command(name = "step-close")]
    StepClose {
        /// Run id or "latest".
        #[arg(default_value = "latest")]
        run: String,
        /// Step number to close.
        #[arg(short, long)]
        index: usize,
    },
    /// Execute one run step in-process, capturing output (used for command steps).
    #[command(name = "exec-step")]
    ExecStep {
        /// Run id or "latest".
        run: String,
        /// Step number to execute.
        index: usize,
    },
    /// Generate a role-based task manifest.
    #[command(alias = "m")]
    Manifest {
        /// Project workspace for the generated manifest.
        #[arg(long, default_value = ".")]
        project: Utf8PathBuf,
        /// Agent id to use for planning.
        #[arg(long, default_value = "claude")]
        planner: String,
        /// Agent id to use for implementation.
        #[arg(long, default_value = "codex")]
        implementer: String,
        /// Agent id to use for review.
        #[arg(long, default_value = "claude")]
        reviewer: String,
        /// Named validation command to include.
        #[arg(long, default_value = "test")]
        command: String,
        /// Prepare a run from the generated manifest.
        #[arg(long)]
        run: bool,
        /// Task goal.
        #[arg(required = true, num_args = 1..)]
        goal: Vec<String>,
    },
    /// Spawn a worker agent in a tmux window.
    Spawn {
        /// Proceed even when a built-in agent CLI is below the pinned version range.
        #[arg(long, env = "NILES_ALLOW_CLI_MISMATCH")]
        allow_cli_mismatch: bool,
        /// Crew task id used for window and metadata names.
        id: String,
        /// Project workspace for the worker.
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
    /// Close a spawned crew worker and remove its metadata.
    #[command(name = "crew-close")]
    CrewClose {
        /// Crew task id to close.
        id: String,
    },
    /// Capture the tail of a worker tmux pane.
    Peek {
        /// Crew task id. Omit when targeting a run step with --run and --index.
        id: Option<String>,
        /// Run id or "latest" for a step window.
        #[arg(long)]
        run: Option<String>,
        /// Step number for a run step window.
        #[arg(short, long)]
        index: Option<usize>,
        /// Number of lines to capture.
        #[arg(short, long, default_value_t = 40)]
        lines: usize,
    },
    /// Send a message to a worker tmux pane.
    Send {
        /// Run id or "latest" for a step window.
        #[arg(long)]
        run: Option<String>,
        /// Step number for a run step window.
        #[arg(short, long)]
        index: Option<usize>,
        /// Crew task id followed by message, or just message when --run/--index are set.
        #[arg(required = true, num_args = 1.., trailing_var_arg = true, value_name = "ID_OR_MESSAGE")]
        target_and_message: Vec<String>,
    },
    /// Wait for the next actionable status-log wake and print it.
    Wait {
        /// Run id. Use --crew instead for a crew worker.
        #[arg(required_unless_present = "crew", conflicts_with = "crew")]
        run: Option<String>,
        /// Crew worker id to wait on.
        #[arg(long, conflicts_with = "run")]
        crew: Option<String>,
        /// Step number to wait for.
        #[arg(short, long)]
        index: Option<usize>,
        /// Poll interval in seconds.
        #[arg(long, default_value_t = 2.0)]
        interval: f64,
        /// Maximum seconds to wait before exiting non-zero. Defaults to 3600 seconds.
        #[arg(long)]
        timeout: Option<f64>,
    },
    /// Resume a persisted run.
    #[command(alias = "re")]
    Resume {
        /// Run id or "latest".
        #[arg(default_value = "latest")]
        run: String,
    },
    /// Inspect a persisted run.
    #[command(alias = "s")]
    Status {
        /// Run id or "latest".
        #[arg(default_value = "latest")]
        run: String,
        /// Print raw JSON state.
        #[arg(long)]
        json: bool,
    },
    /// Watch a persisted run until it finishes.
    #[command(alias = "w")]
    Watch {
        /// Run id or "latest".
        #[arg(default_value = "latest")]
        run: String,
        /// Refresh interval in seconds.
        #[arg(long, default_value_t = 1.0)]
        interval: f64,
        /// Append snapshots instead of clearing the terminal.
        #[arg(long)]
        no_clear: bool,
    },
    /// Show a compact summary of a persisted run.
    #[command(alias = "sh")]
    Show {
        /// Run id or "latest".
        #[arg(default_value = "latest")]
        run: String,
    },
    /// Print stdout or stderr for a run step.
    #[command(alias = "l")]
    Log {
        /// Run id or "latest".
        #[arg(default_value = "latest")]
        run: String,
        /// Step number to inspect. Defaults to the last recorded step.
        #[arg(short, long)]
        step: Option<usize>,
        /// Print stderr instead of stdout.
        #[arg(long)]
        stderr: bool,
        /// Print stdout and stderr.
        #[arg(long)]
        both: bool,
    },
    /// Print the git diff captured after a run step.
    #[command(alias = "d")]
    Diff {
        /// Run id or "latest".
        #[arg(default_value = "latest")]
        run: String,
        /// Step number to inspect. Defaults to the last recorded step.
        #[arg(short, long)]
        step: Option<usize>,
    },
}
