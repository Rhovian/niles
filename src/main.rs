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
mod watch;
mod worker;
mod workspace_manifest;

#[cfg(test)]
mod test_support;

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
        None => session::run()?,
        Some(CommandName::Doctor) => doctor::doctor()?,
        Some(CommandName::Spawn {
            wait,
            id,
            role,
            task_label,
            agent,
            brief,
            checkin,
            mut task,
        }) => {
            // `--wait` and `--checkin <delay>` written after the worker id land in the trailing
            // task text.
            let (trailing_wait, trailing_checkin) = cli::take_leading_dispatch_flags(&mut task);
            let wait = wait || trailing_wait;
            let checkin = checkin.or(trailing_checkin);
            let worker_id = id.clone();
            worker::spawn(id, role, task_label, agent, brief, task, checkin)?;
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
            checkin,
            target_and_message,
        }) => {
            let sent = worker::send(wait, checkin, target_and_message)?;
            if sent.wait_requested {
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
        Some(CommandName::Quiet { id }) => {
            if watch::quiet(&id)? {
                println!("quiet: {id}");
            } else {
                println!("quiet: {id} (no check-in armed)");
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}
