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
