use std::{env, path::PathBuf};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::util::{absolute_path, current_dir_utf8, utf8_path};

pub(super) const NILES_DIR: &str = ".niles";
pub(super) const RUNS_DIR: &str = "runs";
pub(super) const WORKERS_DIR: &str = "worker";
pub(super) const LATEST_POINTER: &str = "latest.json";

pub fn workspace_runs_dir(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    Ok(absolute_path(workspace)?.join(NILES_DIR).join(RUNS_DIR))
}

pub fn workspace_run_dir(workspace: &Utf8Path, run: &str) -> Result<Utf8PathBuf> {
    Ok(workspace_runs_dir(workspace)?.join(run))
}

pub(crate) fn workspace_workers_dir(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    Ok(absolute_path(workspace)?.join(NILES_DIR).join(WORKERS_DIR))
}

pub(crate) fn workspace_worker_dir(workspace: &Utf8Path, worker: &str) -> Result<Utf8PathBuf> {
    Ok(workspace_workers_dir(workspace)?.join(worker))
}

pub(super) fn current_runs_dir() -> Result<Utf8PathBuf> {
    Ok(current_dir_utf8()?.join(NILES_DIR).join(RUNS_DIR))
}

pub(super) fn current_workers_dir() -> Result<Utf8PathBuf> {
    Ok(current_dir_utf8()?.join(NILES_DIR).join(WORKERS_DIR))
}

pub(crate) fn global_index_path() -> Result<Utf8PathBuf> {
    if let Some(home) = env::var_os("NILES_HOME") {
        return Ok(utf8_path(PathBuf::from(home), "NILES_HOME")?
            .join(RUNS_DIR)
            .join("index.json"));
    }

    let home = env::var_os("HOME").context("HOME is not set; cannot write Niles run index")?;
    Ok(utf8_path(PathBuf::from(home), "HOME")?
        .join(NILES_DIR)
        .join(RUNS_DIR)
        .join("index.json"))
}

pub(super) fn pointer_file(run: &str) -> String {
    format!("{run}.json")
}
