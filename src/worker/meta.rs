use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    schema::{self, ArtifactKind},
    util::write_json_pretty,
    wake,
};

use super::{resolve::resolve_worker, validation::validate_id};

const REPORT_FILE: &str = "report.md";

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct WorkerMeta {
    pub(super) id: String,
    pub(super) agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) agent_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) task_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) created_at: Option<DateTime<Utc>>,
    pub(super) project: Utf8PathBuf,
    pub(super) window: String,
    pub(super) brief: Utf8PathBuf,
    pub(super) launch: Utf8PathBuf,
    pub(super) status: Option<Utf8PathBuf>,
}

pub(super) fn write_meta(worker_dir: &Utf8Path, meta: &WorkerMeta) -> Result<()> {
    let path = meta_path(worker_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    }
    write_json_pretty(&path, meta)
}

pub(super) fn read_meta(id: &str) -> Result<WorkerMeta> {
    validate_id(id)?;
    let location = resolve_worker(id)?;
    let path = meta_path(&location.worker_dir);
    let meta = read_meta_if_exists(&location.worker_dir)?
        .with_context(|| format!("worker metadata missing for '{id}' at {path}"))?;
    Ok(meta)
}

pub(super) fn read_meta_if_exists(worker_dir: &Utf8Path) -> Result<Option<WorkerMeta>> {
    let path = meta_path(worker_dir);
    schema::read_optional_json(&path, ArtifactKind::WorkerMetadata)
}

pub(super) fn meta_path(worker_dir: &Utf8Path) -> Utf8PathBuf {
    worker_dir.join("meta.json")
}

pub(super) fn report_path(worker_dir: &Utf8Path) -> Utf8PathBuf {
    worker_dir.join(REPORT_FILE)
}

pub(super) fn metadata_status_path(meta: &WorkerMeta, worker_dir: &Utf8Path) -> Utf8PathBuf {
    match &meta.status {
        Some(status) => status.clone(),
        // Older worker metadata did not stamp `status`; the canonical log lives beside the worker.
        None => wake::status_log_path(worker_dir),
    }
}
