#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod agent_window;
mod agents;
mod build_info;
mod cli;
mod config;
mod doctor;
mod schema;
mod session;
mod store;
mod tmux;
mod util;
mod wait;
mod wake;
mod worker;
mod workspace_manifest;

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, CommandName};

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

    match cli.command {
        None => session::run(cli.lead)?,
        Some(CommandName::Doctor) => doctor::doctor()?,
        Some(CommandName::Spawn {
            wait,
            id,
            role,
            task_label,
            agent,
            brief,
            mut task,
        }) => {
            // `--wait` written after the worker id lands in the trailing task text.
            let wait = wait || cli::take_leading_wait(&mut task);
            let worker_id = id.clone();
            worker::spawn(id, role, task_label, agent, brief, task)?;
            if wait {
                return Ok(
                    wait::wait(vec![worker_id], None, wait::DEFAULT_INTERVAL_SECS, None)?.emit(),
                );
            }
        }
        Some(CommandName::Close {
            id,
            task_label,
            all,
        }) => worker::worker_close(id, task_label, all)?,
        Some(CommandName::Workers) => worker::workers()?,
        Some(CommandName::Report { id }) => worker::report(id)?,
        Some(CommandName::Peek { id, lines }) => worker::peek(id, lines)?,
        Some(CommandName::Send {
            wait,
            target_and_message,
        }) => {
            let sent = worker::send(target_and_message)?;
            if wait || sent.wait_requested {
                return Ok(
                    wait::wait(vec![sent.id], None, wait::DEFAULT_INTERVAL_SECS, None)?.emit(),
                );
            }
        }
        Some(CommandName::Wait {
            worker,
            task,
            interval,
            timeout,
        }) => return Ok(wait::wait(worker, task, interval, timeout)?.emit()),
    }

    Ok(ExitCode::SUCCESS)
}
