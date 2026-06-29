mod analyze;
mod cli;
mod config;
mod context;
mod crew;
mod manifest;
mod process;
mod runner;
mod session;
mod state;
mod store;
mod util;
mod wake;

use anyhow::Result;
use clap::Parser;

use crate::{
    cli::{Cli, CommandName},
    runner::RunSelector,
};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => session::run(cli.supervisor, cli.goal),
        Some(CommandName::Ask { agent, prompt }) => runner::ask(agent, prompt),
        Some(CommandName::Analyze { agent }) => analyze::analyze(agent),
        Some(CommandName::Run { task, watch }) => runner::run(task, watch),
        Some(CommandName::Manifest {
            goal,
            project,
            planner,
            implementer,
            reviewer,
            command,
            run,
            watch,
        }) => manifest::generate(manifest::GenerateOptions {
            goal,
            project,
            planner,
            implementer,
            reviewer,
            command,
            run,
            watch,
        }),
        Some(CommandName::Spawn {
            id,
            project,
            agent,
            brief,
            task,
        }) => crew::spawn(id, project, agent, brief, task),
        Some(CommandName::Peek { id, lines }) => crew::peek(id, lines),
        Some(CommandName::Send { id, message }) => crew::send(id, message),
        Some(CommandName::WatchCrew {
            session,
            interval,
            once,
        }) => wake::watch_crew(session, interval, once),
        Some(CommandName::Resume { run, watch }) => runner::resume(RunSelector::new(run), watch),
        Some(CommandName::Status { run, json }) => runner::status(RunSelector::new(run), json),
        Some(CommandName::Watch {
            run,
            interval,
            no_clear,
        }) => runner::watch(RunSelector::new(run), interval, no_clear),
        Some(CommandName::Show { run }) => runner::show(RunSelector::new(run)),
        Some(CommandName::Log {
            run,
            step,
            stderr,
            both,
        }) => runner::log(RunSelector::new(run), step, stderr, both),
        Some(CommandName::Diff { run, step }) => runner::diff(RunSelector::new(run), step),
    }
}
