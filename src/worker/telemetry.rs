use anyhow::Result;
use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};

use crate::{
    store,
    tmux::{self, TargetState, WindowTarget},
    usage::UsageAttribution,
};

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
    pub(crate) usage_attribution: Option<UsageAttribution>,
    pub(crate) worker_dir: Utf8PathBuf,
    pub(crate) metadata: Utf8PathBuf,
    pub(crate) window: String,
    pub(crate) target_state: TargetState,
    pub(crate) status: Utf8PathBuf,
}

pub(crate) fn live_workers_for_dashboard() -> Result<Vec<WorkerDashboardMeta>> {
    let mut workers = Vec::new();
    for entry in store::resolve_worker_locations()? {
        let worker_dir = entry.location.worker_dir;
        let metadata = meta_path(&worker_dir);
        if !metadata.exists() {
            continue;
        }
        let Some(meta) = read_meta_if_exists(&worker_dir)? else {
            continue;
        };
        let status = metadata_status_path(&meta, &worker_dir);
        let target_state = worker_target_state(&meta.window, meta.project.as_path(), &meta.id);
        workers.push(WorkerDashboardMeta {
            id: entry.id,
            agent: meta.agent,
            agent_family: meta.agent_family,
            model: meta.model,
            effort: meta.effort,
            task_label: meta.task_label,
            created_at: meta.created_at,
            usage_attribution: meta.usage_attribution,
            worker_dir,
            metadata,
            window: meta.window,
            target_state,
            status,
        });
    }
    workers.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(workers)
}

fn worker_target_state(window: &str, project: &camino::Utf8Path, id: &str) -> TargetState {
    match WindowTarget::parse(window) {
        Ok(target) => tmux::target_state(&target, project, id),
        Err(err) => TargetState::Unknown {
            error: format!("{err:#}"),
        },
    }
}
