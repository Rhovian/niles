mod brief;
mod foreground;
mod manager_window;
mod startup;
#[cfg(test)]
mod test_support;
mod tmux_bootstrap;

use std::{env, fs};

use anyhow::{Context, Result};
use camino::Utf8Path;

use crate::{
    dashboard, tmux,
    util::current_dir_utf8,
    workspace_manifest::{self, WorkspaceManifest},
};

use foreground::launch_foreground_agent;
use manager_window::ensure_manager_window;
use tmux_bootstrap::ensure_tmux_session;

pub use brief::SessionMeta;
pub(crate) use brief::read_latest_session as read_latest_manager_session;

pub fn run(manager: Option<String>) -> Result<()> {
    let workspace = current_dir_utf8()?;
    if !ensure_tmux_session(env::var_os("TMUX").as_deref(), &workspace)? {
        return Ok(());
    }

    let manifest = launch_prelude(&workspace, manager.as_deref())?;
    tmux::rename_current_window("niles")?;
    let meta = ensure_manager_window(&workspace, &manifest)?;
    let target = meta
        .window
        .as_deref()
        .context("manager session metadata missing tmux window after launch")?;
    tmux::switch_client(target)?;
    dashboard::run(&workspace)
}

pub fn launch_foreground(manager: Option<String>) -> Result<()> {
    let workspace = current_dir_utf8()?;
    if !ensure_tmux_session(env::var_os("TMUX").as_deref(), &workspace)? {
        return Ok(());
    }

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
