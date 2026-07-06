use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};

use crate::{
    agent_window, store,
    util::{remove_dir_all_if_exists, timestamp_id},
};

use super::meta::WorkerMeta;

const FINAL_PANE_CAPTURE_LINES: usize = 2000;
const FINAL_PANE_FILE: &str = "final-pane.txt";
pub(super) fn final_pane_path(worker_dir: &Utf8Path) -> Utf8PathBuf {
    worker_dir.join(FINAL_PANE_FILE)
}

pub(super) fn capture_final_pane(
    worker_dir: &Utf8Path,
    meta: Option<&WorkerMeta>,
    window_name: &str,
) -> Result<Option<Utf8PathBuf>> {
    if !worker_dir.exists() {
        return Ok(None);
    }

    let text = match meta {
        Some(meta) => agent_window::capture_target(&meta.window, FINAL_PANE_CAPTURE_LINES),
        None => agent_window::capture_window(window_name, FINAL_PANE_CAPTURE_LINES),
    }?;
    if text.is_empty() {
        return Ok(None);
    }
    let path = final_pane_path(worker_dir);
    fs::write(&path, text).with_context(|| format!("failed to write {path}"))?;
    Ok(Some(path))
}

pub(super) fn archive_worker_dir(
    id: &str,
    worker_dir: &Utf8Path,
    archived_at: DateTime<Utc>,
) -> Result<Utf8PathBuf> {
    if !worker_dir.exists() {
        return Ok(worker_dir.to_path_buf());
    }
    let archive_root = archive_root(worker_dir)?;
    fs::create_dir_all(&archive_root)
        .with_context(|| format!("failed to create {archive_root}"))?;
    let archive_dir = archive_root.join(format!("{id}-{}", timestamp_id(&archived_at)));
    move_dir(worker_dir, &archive_dir)?;
    store::register_worker_archive(id, &archive_dir, archived_at)?;
    Ok(archive_dir)
}

fn archive_root(worker_dir: &Utf8Path) -> Result<Utf8PathBuf> {
    let workers_dir = worker_dir
        .parent()
        .with_context(|| format!("worker dir {worker_dir} has no parent"))?;
    Ok(workers_dir.join("archive"))
}

fn move_dir(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(libc::EXDEV) => {
            copy_dir_all(source, destination)?;
            remove_dir_all_if_exists(source)
        }
        Err(err) => Err(err).with_context(|| format!("failed to move {source} to {destination}")),
    }
}

fn copy_dir_all(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    fs::create_dir_all(destination).with_context(|| format!("failed to create {destination}"))?;
    for entry in fs::read_dir(source).with_context(|| format!("failed to read {source}"))? {
        let entry = entry.with_context(|| format!("failed to read entry in {source}"))?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|path| anyhow::anyhow!("path is not UTF-8: {}", path.display()))?;
        let target = destination.join(entry.file_name().to_string_lossy().as_ref());
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {path}"))?;
        if file_type.is_dir() {
            copy_dir_all(&path, &target)?;
        } else {
            fs::copy(&path, &target)
                .with_context(|| format!("failed to copy {path} to {target}"))?;
        }
    }
    Ok(())
}
