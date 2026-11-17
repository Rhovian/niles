use crate::skills::SkillName;
use crate::store::{self, WorkerListEntry};
#[cfg(feature = "remote")]
use crate::telemetry::transcript::TranscriptSource;
use crate::telemetry::{SessionState, Usage};
#[cfg(feature = "remote")]
use crate::tmux::{TargetState, WindowTarget};
use anyhow::Context;

pub(crate) mod agent_status;
mod archive;
mod close;
mod gates;
mod list;
pub(crate) mod meta;
pub(crate) mod pane;
mod report;
mod resolve;
mod skills;
pub(crate) mod snapshot;
mod spawn;
mod stage;
pub(crate) mod usage;
mod validation;
mod worktree;

pub use close::worker_close;
pub use list::workers;
pub(crate) use pane::DEFAULT_PEEK_LINES;
pub use pane::{peek, send};
pub use report::report;
pub(crate) use resolve::resolve_worker as worker_dir;
pub(crate) use skills::skill_workspace;
pub(crate) use snapshot::{ActionableWake, WorkerSnapshot, status_log_len, worker_snapshot};
pub use spawn::{Assignment, SpawnTask, spawn};
pub(crate) use usage::lead_session;
pub use usage::usage;
pub(crate) use usage::worker_session;
pub(crate) use validation::validate_task_label;
pub use worktree::SpawnTree;
pub(crate) use worktree::remove_project_worktrees;

pub(crate) struct StatusWorker {
    pub id: String,
    pub skill: SkillName,
    pub window: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub usage: Option<Usage>,
    pub state: SessionState,
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
    for entry in store::worker_locations(workspace)? {
        if let Some(meta) = meta::read_meta_if_exists(&entry.worker_dir)? {
            labels.push((entry.id, meta.window));
        }
    }
    Ok(labels)
}

/// Worker directories whose metadata is present and readable.
pub(crate) fn live_worker_locations(
    workspace: &camino::Utf8Path,
) -> anyhow::Result<Vec<WorkerListEntry>> {
    let mut live = Vec::new();
    for entry in store::worker_locations(workspace)? {
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
    NeedsAttention,
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
        SessionState::Waiting => WaitHealth::Idle,
        SessionState::NeedsAttention => WaitHealth::NeedsAttention,
        SessionState::Starting | SessionState::Working => WaitHealth::Running,
        SessionState::Unknown => WaitHealth::Unknown,
    }))
}

#[cfg(feature = "remote")]
pub(crate) fn location(
    project: &camino::Utf8Path,
    id: &str,
) -> anyhow::Result<Option<camino::Utf8PathBuf>> {
    if validation::validate_id(id).is_err() {
        return Ok(None);
    }
    let Some(dir) = store::worker_location(project, id)? else {
        return Ok(None);
    };
    Ok(meta::meta_path(&dir).try_exists()?.then_some(dir))
}

#[cfg(feature = "remote")]
pub(crate) struct ChatTarget {
    pub transcript: TranscriptSource,
    pub window: Option<WindowTarget>,
}

#[cfg(feature = "remote")]
pub(crate) fn chat_target(
    project: &camino::Utf8Path,
    id: &str,
) -> anyhow::Result<Option<ChatTarget>> {
    let Some(dir) = location(project, id)? else {
        return Ok(None);
    };
    let Some(meta) = meta::read_meta_if_exists(&dir)? else {
        return Ok(None);
    };
    let window = match resolve::window_state(&meta) {
        TargetState::Live => Some(WindowTarget::parse(&meta.window)?),
        TargetState::OrphanRecovered { actual } => Some(actual),
        TargetState::PaneExited
        | TargetState::WindowDead
        | TargetState::OrphanGone
        | TargetState::Unknown { .. } => None,
    };
    Ok(Some(ChatTarget {
        transcript: TranscriptSource {
            workspace: meta.agent_dir().to_owned(),
            needle: meta::report_path(meta.brief.parent().context("worker brief has no parent")?),
            link: meta.session_link,
            created_at: meta.created_at,
        },
        window,
    }))
}
