use camino::Utf8PathBuf;
use clap::{ArgAction, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    version,
    about,
    infer_subcommands = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: CommandName,
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
    /// Start a new run from a task spec.
    #[command(alias = "r")]
    Run {
        /// YAML task specification.
        task: Utf8PathBuf,
        /// Print compact state snapshots as steps run.
        #[arg(long)]
        watch: bool,
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
        /// Run the generated manifest immediately.
        #[arg(long)]
        run: bool,
        /// Print compact state snapshots during --run.
        #[arg(long)]
        watch: bool,
        /// Task goal.
        #[arg(required = true, num_args = 1..)]
        goal: Vec<String>,
    },
    /// Resume a persisted run.
    #[command(alias = "re")]
    Resume {
        /// Run id or "latest".
        #[arg(default_value = "latest")]
        run: String,
        /// Print compact state snapshots as resumed steps run.
        #[arg(long)]
        watch: bool,
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
