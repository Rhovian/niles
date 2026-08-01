mod agent_window;
mod agents;
mod analyze;
mod build_info;
mod cli;
mod config;
mod dashboard;
mod doctor;
mod schema;
mod session;
mod store;
mod tmux;
mod usage;
mod util;
mod wait;
mod wake;
mod worker;
mod workspace_manifest;

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use crate::cli::{BareSessionMode, Cli, CommandName};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {err:?}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();

    if let Some(mode) = cli.bare_session_mode() {
        match mode {
            BareSessionMode::Resident => session::run(cli.manager)?,
            BareSessionMode::Foreground => session::launch_foreground(cli.manager)?,
        }
        return Ok(ExitCode::SUCCESS);
    }

    match cli.command {
        None => session::run(cli.manager)?,
        Some(CommandName::Analyze { agent }) => analyze::analyze(agent)?,
        Some(CommandName::Doctor) => doctor::doctor()?,
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
        )?,
        Some(CommandName::WorkerClose {
            id,
            task_label,
            all,
        }) => worker::worker_close(id, task_label, all)?,
        Some(CommandName::Workers { usage }) => worker::workers(usage)?,
        Some(CommandName::Report { id }) => worker::report(id)?,
        Some(CommandName::Peek { id, lines }) => worker::peek(id, lines)?,
        Some(CommandName::Send { target_and_message }) => worker::send(target_and_message)?,
        Some(CommandName::Wait {
            worker,
            task,
            interval,
            timeout,
            takeover,
        }) => return Ok(wait::wait(worker, task, interval, timeout, takeover)?.emit()),
    }

    Ok(ExitCode::SUCCESS)
}
