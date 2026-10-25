#[cfg(feature = "remote")]
use super::registry;
#[cfg(feature = "remote")]
use crate::{theme::Theme, tmux, worker, workspace_manifest};
use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use std::{
    env,
    process::{Command, Stdio},
};

#[cfg(feature = "remote")]
pub(crate) enum Start {
    Running,
    Unregistered,
    DirectoryMissing,
    NoManifest,
    NameTaken,
}
#[cfg(feature = "remote")]
pub(crate) enum Stop {
    Stopped,
    NotRunning,
}
#[cfg(feature = "remote")]
pub(crate) enum Closed {
    Closed,
    NotFound,
}
#[cfg(feature = "remote")]
type Run = dyn Fn(&Utf8Path, &[&str]) -> Result<String>;

#[cfg(feature = "remote")]
pub(crate) fn start(name: &str) -> Result<Start> {
    let Some(entry) = registry::find(name)? else {
        return Ok(Start::Unregistered);
    };
    if !entry.path.is_dir() {
        return Ok(Start::DirectoryMissing);
    }
    if workspace_manifest::load(&entry.path)?.is_none() {
        return Ok(Start::NoManifest);
    }
    let session = entry.name.session()?;
    if tmux::project_session(&session)?.is_some_and(|tag| tag != entry.path.as_str()) {
        return Ok(Start::NameTaken);
    }
    if entry.has_live_session()? {
        return Ok(Start::Running);
    }
    if let Err(error) = tmux::open_session(&session, &entry.path, tmux::LeadStart::Unattended) {
        // Another starter can win between lookup and tmux's exclusive session creation.
        if !entry.has_live_session()? {
            return Err(error);
        }
        eprintln!("remote concurrent start: {error:#}");
    }
    tmux::configure_status(&session, &Theme::load()?)?;
    Ok(Start::Running)
}

#[cfg(feature = "remote")]
pub(crate) fn stop(name: &str, run: &Run) -> Result<Stop> {
    let Some(entry) = registry::find(name)? else {
        return Ok(Stop::NotRunning);
    };
    let session = entry.name.session()?;
    if tmux::project_session(&session)?.as_deref() != Some(entry.path.as_str()) {
        return Ok(Stop::NotRunning);
    }
    for worker in worker::live_worker_locations(&entry.path)? {
        if let Err(error) = run(&entry.path, &["close", &worker.id]) {
            // A concurrent desk/lead close satisfies the same postcondition.
            if worker::live_worker_locations(&entry.path)?
                .iter()
                .any(|live| live.id == worker.id)
            {
                return Err(error);
            }
            eprintln!("remote concurrent close: {error:#}");
        }
    }
    if let Err(error) = tmux::kill_session(&session) {
        // A concurrent stop is success only once our tagged session is gone.
        if tmux::project_session(&session)?.as_deref() == Some(entry.path.as_str()) {
            return Err(error);
        }
        eprintln!("remote concurrent stop: {error:#}");
    }
    Ok(Stop::Stopped)
}

#[cfg(feature = "remote")]
pub(crate) fn close_worker(name: &str, id: &str, run: &Run) -> Result<Closed> {
    let Some(entry) = registry::find(name)? else {
        return Ok(Closed::NotFound);
    };
    let Some(worker) = worker::live_worker_locations(&entry.path)?
        .into_iter()
        .find(|worker| worker.id == id)
    else {
        return Ok(Closed::NotFound);
    };
    if let Err(error) = run(&entry.path, &["close", &worker.id]) {
        // A concurrent close may already have archived it; preserve genuine failures.
        if worker::live_worker_locations(&entry.path)?
            .iter()
            .any(|live| live.id == worker.id)
        {
            return Err(error);
        }
        eprintln!("remote concurrent close: {error:#}");
        return Ok(Closed::NotFound);
    }
    Ok(Closed::Closed)
}

/// Runs a worker command as its own process in the worker's project, so the explorer writes
/// nothing the CLI wouldn't.
pub(crate) fn niles_in(project: &Utf8Path, args: &[&str]) -> Result<String> {
    let output = Command::new(env::current_exe().context("failed to find niles executable")?)
        .args(args)
        .current_dir(project)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run niles {}", args.join(" ")))?;
    let stream = if output.status.success() {
        &output.stdout
    } else {
        &output.stderr
    };
    let text = String::from_utf8_lossy(stream)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !output.status.success() {
        bail!(text);
    }
    Ok(text)
}

#[cfg(all(test, feature = "remote"))]
mod tests;
