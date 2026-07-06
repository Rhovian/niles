mod brief;
mod foreground;
mod startup;
#[cfg(test)]
mod test_support;
mod tmux_bootstrap;

use std::{env, fs};

use anyhow::{Context, Result};
use camino::Utf8Path;

use crate::{
    util::current_dir_utf8,
    workspace_manifest::{self, WorkspaceManifest},
};

use foreground::launch_foreground_agent;
use tmux_bootstrap::ensure_tmux_session;

pub use brief::SessionMeta;

pub fn run(manager: Option<String>) -> Result<()> {
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
