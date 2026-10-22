#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod agent_window;
mod agents;
mod build_info;
mod cli;
mod compat;
mod config;
mod doctor;
mod duration;
mod models;
mod projects;
#[cfg(feature = "remote")]
mod remote;
mod session;
mod store;
mod telemetry;
mod theme;
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
        Some(CommandName::Agent { command }) => worker::agent_status::run(command)?,
        None => projects::explorer::home()?,
        Some(CommandName::Explorer) => projects::explorer::run()?,
        Some(CommandName::Panel { panel }) => projects::panels::render(panel)?,
        Some(CommandName::Lead) => session::run()?,
        Some(CommandName::Status { line }) => {
            return Ok(match projects::status::run(line) {
                Ok(text) => {
                    println!("{text}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    println!(
                        "niles: {}",
                        format!("{error:#}").replace('\n', " ").replace('#', "##")
                    );
                    ExitCode::FAILURE
                }
            });
        }
        Some(CommandName::Remote { port, command }) => {
            #[cfg(feature = "remote")]
            match command {
                Some(cli::RemoteCommand::Uninstall) => remote::setup::uninstall()?,
                Some(cli::RemoteCommand::Doctor) => remote::setup::doctor()?,
                Some(cli::RemoteCommand::Init { subject }) => remote::push::init(&subject)?,
                Some(cli::RemoteCommand::Setup {
                    port,
                    web_dir,
                    allow_user,
                    subject,
                }) => remote::setup::run(&web_dir, allow_user, subject.as_deref(), port)?,
                None => remote::run(port)?,
            }
            #[cfg(not(feature = "remote"))]
            {
                let _ = (port, command);
                anyhow::bail!("built without remote; reinstall with --features remote");
            }
        }
        Some(CommandName::Doctor) => doctor::doctor()?,
        Some(CommandName::Compat { families }) => return compat::compat(&families),
        Some(CommandName::Prune { older_than, apply }) => projects::prune::run(older_than, apply)?,
        Some(CommandName::Spawn {
            wait,
            id,
            assignment,
            task_label,
            agent,
            tree,
            worktree,
            base,
            checkin,
            message,
        }) => {
            let task = message.resolve()?;
            let tree = match worktree {
                Some(branch) => Some(worker::SpawnTree::Worktree { branch, base }),
                None => tree.map(worker::SpawnTree::Path),
            };
            worker::spawn(id, assignment, task_label, agent, task, checkin, tree)?;
            if wait {
                return Ok(wait::wait(wait::DEFAULT_INTERVAL, wait::DEFAULT_TIMEOUT)?.emit());
            }
        }
        Some(CommandName::Close {
            id,
            task_label,
            all,
        }) => worker::worker_close(id, task_label, all)?,
        Some(CommandName::Retier { id, agent }) => worker::retier(id, agent)?,
        Some(CommandName::Log { id, decision }) => session::log::run(&id, &decision.join(" "))?,
        Some(CommandName::Workers) => worker::workers()?,
        Some(CommandName::Usage) => worker::usage()?,
        Some(CommandName::Models) => models::models()?,
        Some(CommandName::Report { id }) => worker::report(id)?,
        Some(CommandName::Peek { id, lines, quiet }) => worker::peek(id, lines, quiet)?,
        Some(CommandName::Send {
            wait,
            checkin,
            id,
            message,
        }) => {
            let message = message.resolve()?;
            worker::send(wait, checkin, id, message)?;
            if wait {
                return Ok(wait::wait(wait::DEFAULT_INTERVAL, wait::DEFAULT_TIMEOUT)?.emit());
            }
        }
        Some(CommandName::Wait { interval, timeout }) => {
            return Ok(wait::wait(interval, timeout)?.emit());
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
