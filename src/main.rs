#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod agent_window;
mod agents;
mod build_info;
mod cli;
mod config;
mod doctor;
mod duration;
mod models;
mod projects;
mod session;
mod store;
mod telemetry;
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
        None => projects::explorer::home()?,
        Some(CommandName::Explorer) => projects::explorer::run()?,
        Some(CommandName::Lead) => session::run()?,
        Some(CommandName::Status { line }) => {
            return Ok(match projects::status::run(line) {
                Ok(text) => {
                    println!("{text}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    println!("niles: {}", format!("{error:#}").replace('\n', " "));
                    ExitCode::FAILURE
                }
            });
        }
        Some(CommandName::Doctor) => doctor::doctor()?,
        Some(CommandName::Prune { older_than, apply }) => projects::prune::run(older_than, apply)?,
        Some(CommandName::Spawn {
            wait,
            id,
            role,
            task_label,
            agent,
            tree,
            worktree,
            base,
            checkin,
            message,
        }) => {
            let task = message.resolve()?;
            let worker_id = id.clone();
            let tree = match worktree {
                Some(branch) => Some(worker::SpawnTree::Worktree { branch, base }),
                None => tree.map(worker::SpawnTree::Path),
            };
            worker::spawn(id, role, task_label, agent, task, checkin, tree)?;
            if wait {
                return Ok(wait::wait(
                    wait::WaitOn::Workers(vec![worker_id]),
                    wait::DEFAULT_INTERVAL,
                    wait::DEFAULT_TIMEOUT,
                )?
                .emit());
            }
        }
        Some(CommandName::Close {
            id,
            task_label,
            all,
        }) => worker::worker_close(id, task_label, all)?,
        Some(CommandName::Workers) => worker::workers()?,
        Some(CommandName::Usage) => worker::usage()?,
        Some(CommandName::Models) => models::models()?,
        Some(CommandName::Report { id }) => worker::report(id)?,
        Some(CommandName::Peek { id, lines }) => worker::peek(id, lines)?,
        Some(CommandName::Send {
            wait,
            checkin,
            id,
            message,
        }) => {
            let message = message.resolve()?;
            let sent = worker::send(wait, checkin, id, message)?;
            if sent.wait_requested {
                return Ok(wait::wait(
                    wait::WaitOn::Workers(vec![sent.id]),
                    wait::DEFAULT_INTERVAL,
                    wait::DEFAULT_TIMEOUT,
                )?
                .emit());
            }
        }
        Some(CommandName::Wait {
            worker,
            task,
            interval,
            timeout,
        }) => {
            let on = match task {
                Some(label) => wait::WaitOn::Task(label),
                None => wait::WaitOn::Workers(worker),
            };
            return Ok(wait::wait(on, interval, timeout)?.emit());
        }
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
