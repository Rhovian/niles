use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{store, telemetry::SessionLink};

use super::role::WorkerRole;

const REPORT_FILE: &str = "report.md";

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct WorkerMeta {
    pub(super) id: String,
    pub(super) role: WorkerRole,
    pub(super) agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) agent_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) task_label: Option<String>,
    pub(super) created_at: DateTime<Utc>,
    pub(super) project: Utf8PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) tree: Option<Utf8PathBuf>,
    pub(super) window: String,
    pub(super) brief: Utf8PathBuf,
    pub(super) launch: Utf8PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) session_link: Option<SessionLink>,
}

impl WorkerMeta {
    pub(super) fn agent_dir(&self) -> &Utf8Path {
        match &self.tree {
            Some(tree) => tree,
            None => &self.project,
        }
    }
}

pub(super) fn write_meta(worker_dir: &Utf8Path, meta: &WorkerMeta) -> Result<()> {
    let path = meta_path(worker_dir);
    store::write_json(&path, meta)
}

pub(super) fn read_meta(worker_dir: &Utf8Path) -> Result<WorkerMeta> {
    let path = meta_path(worker_dir);
    let meta = read_meta_if_exists(worker_dir)?
        .with_context(|| format!("worker metadata missing at {path}"))?;
    Ok(meta)
}

pub(super) fn read_meta_if_exists(worker_dir: &Utf8Path) -> Result<Option<WorkerMeta>> {
    let path = meta_path(worker_dir);
    store::read_optional_json(&path)
}

pub(super) fn meta_path(worker_dir: &Utf8Path) -> Utf8PathBuf {
    worker_dir.join("meta.json")
}

pub(super) fn report_path(worker_dir: &Utf8Path) -> Utf8PathBuf {
    worker_dir.join(REPORT_FILE)
}
