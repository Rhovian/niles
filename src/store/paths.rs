use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};

use crate::util::{absolute_path, current_dir_utf8};

pub(super) const NILES_DIR: &str = ".niles";
pub(super) const WORKERS_DIR: &str = "worker";

fn workspace_workers_dir(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    Ok(absolute_path(workspace)?.join(NILES_DIR).join(WORKERS_DIR))
}

pub(crate) fn workspace_worker_dir(workspace: &Utf8Path, worker: &str) -> Result<Utf8PathBuf> {
    Ok(workspace_workers_dir(workspace)?.join(worker))
}

pub(super) fn current_workers_dir() -> Result<Utf8PathBuf> {
    Ok(current_dir_utf8()?.join(NILES_DIR).join(WORKERS_DIR))
}
