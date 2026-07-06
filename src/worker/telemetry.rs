use anyhow::Result;
use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};

use crate::store;

use super::meta::{meta_path, metadata_status_path, read_meta_if_exists};

#[derive(Debug, Clone)]
pub(crate) struct WorkerDashboardMeta {
    pub(crate) id: String,
    pub(crate) agent: String,
    pub(crate) agent_family: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) task_label: Option<String>,
    pub(crate) created_at: Option<DateTime<Utc>>,
    pub(crate) window: String,
    pub(crate) status: Utf8PathBuf,
}

pub(crate) fn live_workers_for_dashboard() -> Result<Vec<WorkerDashboardMeta>> {
    let mut workers = Vec::new();
    for entry in store::resolve_worker_locations()? {
        if !meta_path(&entry.location.worker_dir).exists() {
            continue;
        }
        let Some(meta) = read_meta_if_exists(&entry.location.worker_dir)? else {
            continue;
        };
        let status = metadata_status_path(&meta, &entry.location.worker_dir);
        workers.push(WorkerDashboardMeta {
            id: entry.id,
            agent: meta.agent,
            agent_family: meta.agent_family,
            model: meta.model,
            effort: meta.effort,
            task_label: meta.task_label,
            created_at: meta.created_at,
            window: meta.window,
            status,
        });
    }
    workers.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(workers)
}
