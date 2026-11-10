use crate::store::{self, atomic_write};
use crate::tmux::{self, WindowTarget};
use crate::util::timestamp_id;
use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use std::fs;

const FINAL_PANE_CAPTURE_LINES: usize = 2000;
const FINAL_PANE_FILE: &str = "final-pane.txt";
pub(super) fn final_pane_path(worker_dir: &Utf8Path) -> Utf8PathBuf {
    worker_dir.join(FINAL_PANE_FILE)
}

pub(super) fn capture_final_pane(
    worker_dir: &Utf8Path,
    target: &WindowTarget,
) -> Result<Option<Utf8PathBuf>> {
    if !worker_dir.exists() {
        return Ok(None);
    }

    let text = tmux::capture_pane(&tmux::TmuxTarget::window(target), FINAL_PANE_CAPTURE_LINES)?;
    if text.is_empty() {
        return Ok(None);
    }
    let path = final_pane_path(worker_dir);
    atomic_write(&path, text)?;
    Ok(Some(path))
}

pub(super) fn archive_worker_dir(
    workspace: &Utf8Path,
    id: &str,
    worker_dir: &Utf8Path,
    archived_at: DateTime<Utc>,
) -> Result<Utf8PathBuf> {
    if !worker_dir.exists() {
        return Ok(worker_dir.to_path_buf());
    }
    let archive_root = store::archive_dir(workspace)?;
    fs::create_dir_all(&archive_root)
        .with_context(|| format!("failed to create {archive_root}"))?;
    let archive_dir = archive_root.join(format!("{id}-{}", timestamp_id(&archived_at)));
    store::move_dir(worker_dir, &archive_dir)?;
    Ok(archive_dir)
}
