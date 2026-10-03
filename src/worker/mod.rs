use anyhow::Context;

mod archive;
mod close;
mod list;
mod meta;
mod pane;
mod report;
mod resolve;
mod role;
mod snapshot;
mod spawn;
mod usage;
mod validation;
mod worktree;

pub use close::worker_close;
pub use list::workers;
pub use pane::{peek, send};
pub use report::report;
pub use resolve::window_is_gone;
pub use role::WorkerRole;
pub use spawn::spawn;
pub use usage::usage;
pub use worktree::SpawnTree;

pub(crate) use close::select_worker_ids_by_task;
pub(crate) use pane::DEFAULT_PEEK_LINES;
pub(crate) use resolve::resolve_worker as worker_dir;
pub(crate) use snapshot::{ActionableWake, WorkerSnapshot, status_log_len, worker_snapshot};
pub(crate) use usage::{lead_usage, worker_usage};

pub(crate) struct StatusWorker {
    pub id: String,
    pub window: String,
    pub model: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub usage: Option<crate::telemetry::Usage>,
}

pub(crate) fn status_workers(workspace: &camino::Utf8Path) -> anyhow::Result<Vec<StatusWorker>> {
    worker_snapshot(workspace)?
        .into_iter()
        .map(|worker| {
            let usage = worker_usage(&worker)?;
            let meta = worker.meta.as_ref().with_context(|| {
                format!(
                    "worker {} metadata is unreadable: {:?}",
                    worker.id, worker.read_error
                )
            })?;
            Ok(StatusWorker {
                id: worker.id.clone(),
                window: meta.window.clone(),
                model: meta.model.clone().unwrap_or_else(|| meta.agent.clone()),
                created_at: meta.created_at,
                usage,
            })
        })
        .collect()
}
pub(crate) use validation::validate_task_label;
