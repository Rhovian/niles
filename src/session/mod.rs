mod brief;
mod foreground;
mod startup;
#[cfg(test)]
mod test_support;

use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};

use crate::{
    store, tmux,
    util::{current_dir_utf8, dated_directories, parse_timestamp_id},
    workspace_manifest::{self, WorkspaceManifest},
};

pub(crate) use brief::{SessionMeta, latest_lead, live_lead};
use foreground::launch_foreground_agent;

/// Turns the current project's lead pane into the manager agent.
///
/// Niles creates one session per project, named after its registry entry and tagged with its path.
/// Its lead and workers run in that session.
pub fn run() -> Result<()> {
    let workspace = current_dir_utf8()?;
    // Fails here, before any manifest prompting, so being outside tmux costs one line and no setup.
    tmux::current_session()?;
    let Some(manifest) = launch_prelude(&workspace)? else {
        return Ok(());
    };
    launch_foreground_agent(&workspace, &manifest)
}

fn launch_prelude(workspace: &Utf8Path) -> Result<Option<WorkspaceManifest>> {
    let worker_dir = store::workers_dir(workspace);
    fs::create_dir_all(&worker_dir).with_context(|| format!("failed to create {worker_dir}"))?;

    workspace_manifest::ensure_interactive(workspace)
}

pub(crate) fn sessions_dir(workspace: &Utf8Path) -> Utf8PathBuf {
    workspace.join(store::paths::NILES_DIR).join("sessions")
}

/// Every lead session directory, oldest first, with the time it was created.
pub(crate) fn session_dirs(workspace: &Utf8Path) -> Result<Vec<(DateTime<Utc>, Utf8PathBuf)>> {
    dated_directories(&sessions_dir(workspace), parse_timestamp_id)
}
