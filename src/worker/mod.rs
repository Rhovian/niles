use anyhow::Context;

mod archive;
mod close;
mod list;
mod meta;
mod pane;
mod report;
mod resolve;
mod retier;
mod role;
mod snapshot;
mod spawn;
pub(crate) mod usage;
mod validation;
mod worktree;

pub use close::worker_close;
pub use list::workers;
pub use pane::{peek, send};
pub use report::report;
pub use retier::retier;
pub use role::WorkerRole;
pub use spawn::{Assignment, spawn};
pub use usage::usage;
pub use worktree::SpawnTree;

pub(crate) use pane::DEFAULT_PEEK_LINES;
pub(crate) use resolve::resolve_worker as worker_dir;
pub(crate) use snapshot::{ActionableWake, WorkerSnapshot, status_log_len, worker_snapshot};
pub(crate) use usage::lead_session;
pub(crate) use usage::worker_session;
pub(crate) use validation::validate_task_label;

pub(crate) struct StatusWorker {
    pub id: String,
    pub role: WorkerRole,
    pub window: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub usage: Option<crate::telemetry::Usage>,
    pub state: crate::telemetry::SessionState,
}

pub(crate) fn status_workers(workspace: &camino::Utf8Path) -> anyhow::Result<Vec<StatusWorker>> {
    worker_snapshot(workspace)?
        .into_iter()
        .map(|worker| {
            let record = usage::worker_session(&worker, chrono::Utc::now())?;
            let meta = worker.meta.as_ref().with_context(|| {
                format!(
                    "worker {} metadata is unreadable: {:?}",
                    worker.id, worker.read_error
                )
            })?;
            Ok(StatusWorker {
                id: worker.id.clone(),
                role: meta.role,
                window: meta.window.clone(),
                created_at: meta.created_at,
                usage: record.usage,
                state: record.state,
            })
        })
        .collect()
}

/// Live worker directory identities and their recorded windows, without activity or logs.
pub(crate) fn window_labels(workspace: &camino::Utf8Path) -> anyhow::Result<Vec<(String, String)>> {
    let mut labels = Vec::new();
    for entry in crate::store::worker_locations(workspace)? {
        if let Some(meta) = meta::read_meta_if_exists(&entry.worker_dir)? {
            labels.push((entry.id, meta.window));
        }
    }
    Ok(labels)
}

/// Worker directories whose metadata is present and readable.
pub(crate) fn live_worker_locations(
    workspace: &camino::Utf8Path,
) -> anyhow::Result<Vec<crate::store::WorkerListEntry>> {
    let mut live = Vec::new();
    for entry in crate::store::worker_locations(workspace)? {
        if meta::read_meta_if_exists(&entry.worker_dir)?.is_some() {
            validation::validate_id(&entry.id)?;
            live.push(entry);
        }
    }
    Ok(live)
}
