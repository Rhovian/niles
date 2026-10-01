mod brief;
mod foreground;
mod startup;
#[cfg(test)]
mod test_support;

use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    store, tmux,
    util::current_dir_utf8,
    workspace_manifest::{self, WorkspaceManifest},
};

pub(crate) use brief::{SessionMeta, live_lead};
use foreground::launch_foreground_agent;

/// Turns the current tmux pane into the manager agent.
///
/// Niles does not create, name, pin or attach tmux sessions. The operator's current session is
/// the session, which is what lets worker placement be a fact rather than a resolution strategy.
pub fn run() -> Result<()> {
    let workspace = current_dir_utf8()?;
    // Fails here, before any manifest prompting, so being outside tmux costs one line and no setup.
    tmux::current_session()?;
    let manifest = launch_prelude(&workspace)?;
    launch_foreground_agent(&workspace, &manifest)
}

fn launch_prelude(workspace: &Utf8Path) -> Result<WorkspaceManifest> {
    let worker_dir = store::workers_dir(workspace);
    fs::create_dir_all(&worker_dir).with_context(|| format!("failed to create {worker_dir}"))?;

    workspace_manifest::ensure_interactive(workspace)
}

pub(crate) fn sessions_dir(workspace: &Utf8Path) -> Utf8PathBuf {
    workspace.join(store::paths::NILES_DIR).join("sessions")
}
