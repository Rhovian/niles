mod brief;
mod foreground;
mod startup;
#[cfg(test)]
mod test_support;

use std::fs;

use anyhow::{Context, Result};
use camino::Utf8Path;

use crate::{
    tmux,
    util::current_dir_utf8,
    workspace_manifest::{self, WorkspaceManifest},
};

use foreground::launch_foreground_agent;

pub use brief::SessionMeta;

/// Turns the current tmux pane into the manager agent.
///
/// Niles does not create, name, pin or attach tmux sessions. The operator's current session is
/// the session, which is what lets worker placement be a fact rather than a resolution strategy.
pub fn run(manager: Option<String>) -> Result<()> {
    let workspace = current_dir_utf8()?;
    // Fails here, before any manifest prompting, so being outside tmux costs one line and no setup.
    tmux::current_session()?;
    let manifest = launch_prelude(&workspace, manager.as_deref())?;
    launch_foreground_agent(&workspace, &manifest)
}

fn launch_prelude(
    workspace: &Utf8Path,
    manager_override: Option<&str>,
) -> Result<WorkspaceManifest> {
    let worker_dir = workspace.join(".niles").join("worker");
    fs::create_dir_all(&worker_dir).with_context(|| format!("failed to create {worker_dir}"))?;

    let mut defaults = WorkspaceManifest::default();
    if let Some(manager) = manager_override {
        defaults.manager = manager.to_owned();
    }

    workspace_manifest::ensure_interactive(workspace, &defaults)
}
