mod agent_window;
mod agents;
mod analyze;
mod build_info;
mod cli;
mod config;
mod context;
mod dashboard;
mod display;
mod doctor;
mod process;
mod runner;
mod schema;
mod session;
mod state;
mod store;
mod tmux;
mod usage;
mod util;
mod wait;
mod wake;
mod worker;
mod workspace_manifest;

use anyhow::Result;
use clap::Parser;

use crate::{
    cli::{BareSessionMode, Cli, CommandName},
    runner::RunSelector,
};

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(mode) = cli.bare_session_mode() {
        return match mode {
            BareSessionMode::Resident => session::run(cli.manager),
            BareSessionMode::Foreground => session::launch_foreground(cli.manager),
        };
    }

    match cli.command {
        None => session::run(cli.manager),
        Some(CommandName::Analyze { agent }) => analyze::analyze(agent),
        Some(CommandName::Doctor) => doctor::doctor(),
        Some(CommandName::Run {
            allow_cli_mismatch,
            task,
        }) => runner::run(task, allow_cli_mismatch),
        Some(CommandName::Step { run, index }) => runner::step(RunSelector::new(run), index),
        Some(CommandName::StepAdd {
            run,
            agent,
            command,
            role,
            task,
        }) => runner::step_add(RunSelector::new(run), agent, command, role, task),
        Some(CommandName::StepClose { run, index }) => {
            runner::step_close(RunSelector::new(run), index)
        }
        Some(CommandName::ExecStep { run, index }) => {
            runner::exec_step(RunSelector::new(run), index)
        }
        Some(CommandName::Spawn {
            allow_cli_mismatch,
            id,
            task_label,
            project,
            agent,
            brief,
            task,
        }) => worker::spawn(
            id,
            task_label,
            project,
            agent,
            brief,
            task,
            allow_cli_mismatch,
        ),
        Some(CommandName::WorkerClose {
            id,
            task_label,
            all,
        }) => worker::worker_close(id, task_label, all),
        Some(CommandName::Workers { usage }) => worker::workers(usage),
        Some(CommandName::Report { id }) => worker::report(id),
        Some(CommandName::Peek {
            id,
            run,
            index,
            lines,
        }) => worker::peek(id, run, index, lines),
        Some(CommandName::Send {
            run,
            index,
            target_and_message,
        }) => worker::send(run, index, target_and_message),
        Some(CommandName::Wait {
            run,
            worker,
            task,
            index,
            interval,
            timeout,
        }) => wait::wait(run, worker, task, index, interval, timeout),
        Some(CommandName::Resume { run }) => runner::resume(RunSelector::new(run)),
        Some(CommandName::Status { run, json, usage }) => {
            runner::status(RunSelector::new(run), json, usage)
        }
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
