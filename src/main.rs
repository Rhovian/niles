#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")
)]

mod agent_window;
mod agents;
mod build_info;
mod cli;
mod commands;
mod compat;
mod config;
mod doctor;
mod duration;
mod harness;
mod mcp;
mod models;
mod projects;
#[cfg(feature = "remote")]
mod remote;
mod selection;
mod session;
mod skills;
mod store;
mod telemetry;
mod theme;
mod tmux;
mod update;
mod util;
mod wait;
mod wake;
mod watch;
mod worker;
mod workspace_manifest;

#[cfg(test)]
mod test_support;

use crate::cli::{Cli, CommandName};
use anyhow::Result;
use clap::Parser;
use commands::Output;
use std::io;
use std::process::{self, ExitCode};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {err:?}");
            // Runtime drop waits for blocking jobs; fatal errors must exit immediately.
            process::exit(1)
        }
    }
}

async fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let selection = || {
        cli.setup
            .as_deref()
            .map(str::parse::<workspace_manifest::setup::SetupSelection>)
            .transpose()
    };

    let mut output = Output {
        out: &mut io::stdout(),
        err: &mut io::stderr(),
    };
    match cli.command {
        Some(CommandName::Agent { command }) => worker::agent_status::run(command, &mut output)?,
        Some(CommandName::Mcp) => mcp::run(cli.setup).await?,
        None => projects::explorer::home()?,
        Some(CommandName::Acp { dir, prompt }) => harness::acp::run(&dir, prompt).await?,
        Some(CommandName::Cancel(args)) => worker::pane::cancel(args.id, &mut output)?,
        Some(CommandName::Approve(args)) => args.run()?,
        Some(CommandName::Resume { id }) => worker::pane::resume(id)?,
        Some(CommandName::Explorer) => projects::explorer::run()?,
        Some(CommandName::Panel { panel }) => projects::panels::render(panel)?,
        Some(CommandName::Start {
            keep_skills,
            agent,
            launch,
        }) => {
            session::run(
                if keep_skills {
                    tmux::LeadStart::Unattended
                } else {
                    tmux::LeadStart::Attended
                },
                agent.as_deref(),
                launch,
                selection()?,
            )
            .await?
        }
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
                Some(cli::RemoteCommand::Init { subject }) => remote::push::init(&subject)?,
                None => remote::run(port).await?,
            }
            #[cfg(not(feature = "remote"))]
            {
                let _ = (port, command);
                anyhow::bail!("built without remote; reinstall with --features remote");
            }
        }
        Some(CommandName::Skills { command }) => match command {
            None => skills::list(selection()?.as_ref())?,
            Some(command) => command.run()?,
        },
        Some(CommandName::Setup(options)) => {
            anyhow::ensure!(
                cli.setup.is_none(),
                "setup does not accept --setup or NILES_SETUP"
            );
            projects::onboard::unattended::run(*options)?;
        }
        Some(CommandName::Doctor) => doctor::doctor(selection()?.as_ref())?,
        Some(CommandName::Update { channel }) => update::run(channel)?,
        Some(CommandName::Compat { families }) => {
            return compat::compat(&families, selection()?.as_ref());
        }
        Some(CommandName::Prune { older_than, apply }) => projects::prune::run(older_than, apply)?,
        Some(CommandName::Spawn { args, message }) => {
            let worker = args.run(message.resolve()?, selection()?.as_ref(), &mut output)?;
            cli::spawn_hints(output.out, worker)?;
        }
        Some(CommandName::Close(args)) => args.run(&mut output)?,
        Some(CommandName::Log { id, decision }) => session::log::run(&id, &decision.join(" "))?,
        Some(CommandName::Workers) => worker::print_json(&worker::workers(&mut output)?)?,
        Some(CommandName::Usage) => worker::usage()?,
        Some(CommandName::Models) => models::models()?,
        Some(CommandName::Report(args)) => worker::report(args.id, &mut output)?,
        Some(CommandName::Peek(args)) => args.run(&mut output)?,
        Some(CommandName::Send { args, message }) => {
            let id = args.run(message.resolve()?, selection()?.as_ref(), &mut output)?;
            cli::send_hints(output.out, &id)?;
        }
        Some(CommandName::Wait(args)) => return args.run(|| false)?.emit(output.out),
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
