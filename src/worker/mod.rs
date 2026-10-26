use anyhow::Context;

pub(crate) mod agent_status;
mod archive;
mod close;
mod list;
mod meta;
mod pane;
mod report;
mod resolve;
mod retier;
mod skills;
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
    pub skill: crate::skills::SkillName,
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
                skill: meta.skill.clone(),
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

#[cfg(feature = "remote")]
pub(crate) fn model_labels(workspace: &camino::Utf8Path) -> anyhow::Result<Vec<(String, String)>> {
    let mut labels = Vec::new();
    for entry in live_worker_locations(workspace)? {
        let meta = meta::read_meta(&entry.worker_dir)?;
        let model = match meta.model {
            Some(model) => model,
            None => meta.agent,
        };
        labels.push((entry.id, model));
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

#[derive(Clone, Copy)]
pub(crate) enum WaitHealth {
    Gone,
    Idle,
    Running,
    Unknown,
}

pub(crate) fn wait_health(
    dir: &camino::Utf8Path,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Option<WaitHealth>> {
    let Some(meta) = meta::read_meta_if_exists(dir)? else {
        return Ok(None);
    };
    if resolve::window_state(&meta).is_gone() {
        return Ok(Some(WaitHealth::Gone));
    }
    let mut snapshot = WorkerSnapshot::new(meta.id.clone(), dir.to_path_buf(), None);
    snapshot.meta = Some(meta);
    Ok(Some(match worker_session(&snapshot, now)?.state {
        crate::telemetry::SessionState::Waiting => WaitHealth::Idle,
        crate::telemetry::SessionState::Starting | crate::telemetry::SessionState::Working => {
            WaitHealth::Running
        }
        crate::telemetry::SessionState::Unknown => WaitHealth::Unknown,
    }))
}

#[cfg(feature = "remote")]
pub(crate) struct ChatTarget {
    pub transcript: crate::telemetry::transcript::TranscriptSource,
    pub window: Option<crate::tmux::WindowTarget>,
}

#[cfg(feature = "remote")]
pub(crate) fn chat_target(
    project: &camino::Utf8Path,
    id: &str,
) -> anyhow::Result<Option<ChatTarget>> {
    let Some(entry) = live_worker_locations(project)?
        .into_iter()
        .find(|entry| entry.id == id)
    else {
        return Ok(None);
    };
    let meta = meta::read_meta(&entry.worker_dir)?;
    let window = match resolve::window_state(&meta) {
        crate::tmux::TargetState::Live => Some(crate::tmux::WindowTarget::parse(&meta.window)?),
        crate::tmux::TargetState::OrphanRecovered { actual } => Some(actual),
        crate::tmux::TargetState::PaneExited
        | crate::tmux::TargetState::WindowDead
        | crate::tmux::TargetState::OrphanGone
        | crate::tmux::TargetState::Unknown { .. } => None,
    };
    Ok(Some(ChatTarget {
        transcript: crate::telemetry::transcript::TranscriptSource {
            workspace: meta.agent_dir().to_owned(),
            needle: meta::report_path(meta.brief.parent().context("worker brief has no parent")?),
            link: meta.session_link,
            created_at: meta.created_at,
        },
        window,
    }))
}
