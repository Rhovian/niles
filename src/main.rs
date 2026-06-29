mod analyze;
mod cli;
mod process;
mod runner;
mod spec;
mod state;
mod store;

use anyhow::Result;
use clap::Parser;

use crate::{
    cli::{Cli, CommandName},
    runner::RunSelector,
};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        CommandName::Ask { agent, prompt } => runner::ask(agent, prompt),
        CommandName::Analyze { agent } => analyze::analyze(agent),
        CommandName::Run { task } => runner::run(task),
        CommandName::Resume { run } => runner::resume(RunSelector::new(run)),
        CommandName::Status { run } => runner::status(RunSelector::new(run)),
        CommandName::Show { run } => runner::show(RunSelector::new(run)),
        CommandName::Log {
            run,
            step,
            stderr,
            both,
        } => runner::log(RunSelector::new(run), step, stderr, both),
        CommandName::Diff { run, step } => runner::diff(RunSelector::new(run), step),
    }
}
