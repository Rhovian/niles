use std::io::Write;

use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::{tmux::TargetState, util::current_dir_utf8, wake, watch};

use super::snapshot::{WorkerSnapshot, worker_snapshot};

/// Renders the workspace's live workers as one compact JSON object.
///
/// Every fact comes from [`worker_snapshot`], which is also what the watcher reads: a listing that
/// enumerated workers its own way would be a second answer to "which workers are live", and the
/// two would drift.
pub fn workers() -> Result<()> {
    let snapshots = worker_snapshot(&current_dir_utf8()?)?;
    for worker in &snapshots {
        if let Some(error) = &worker.read_error {
            eprintln!(
                "worker {} metadata is unreadable; remove its directory to recover: {error}",
                worker.id
            );
        }
    }
    let workers = snapshots
        .iter()
        .map(WorkerOutput::read)
        .collect::<Result<Vec<_>>>()?;
    print_json(&WorkersOutput { workers })
}

pub(super) fn print_json(value: &impl Serialize) -> Result<()> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer(&mut output, value).context("failed to serialize JSON")?;
    writeln!(output).context("failed to write JSON")
}

#[derive(Serialize)]
struct WorkersOutput {
    workers: Vec<WorkerOutput>,
}

#[derive(Serialize)]
struct WorkerOutput {
    id: String,
    skill: Option<crate::skills::SkillName>,
    agent: Option<String>,
    tree: Option<Utf8PathBuf>,
    task_label: Option<String>,
    started_at: Option<DateTime<Utc>>,
    window: Option<TargetState>,
    wake: Option<WakeState>,
    last_status: Option<String>,
    state: Option<crate::telemetry::SessionState>,
    checkin: Option<CheckinOutput>,
    error: Option<String>,
}

impl WorkerOutput {
    fn read(worker: &WorkerSnapshot) -> Result<Self> {
        let meta = worker.meta.as_ref();
        let wake = match meta {
            Some(_) if has_pending_wake(worker) => Some(WakeState::Pending),
            Some(_) => Some(WakeState::Clear),
            None => None,
        };
        let checkin = watch::Checkin::read(&worker.worker_dir)?.map(|checkin| CheckinOutput {
            deadline: checkin.deadline,
        });

        let (state, error) = match meta {
            None => (None, worker.read_error.clone()),
            Some(_) => match super::worker_session(worker, Utc::now()) {
                Ok(activity) => (Some(activity.state.displayed()), worker.read_error.clone()),
                Err(err) => (None, Some(format!("failed to read activity: {err:#}"))),
            },
        };
        Ok(Self {
            id: worker.id.clone(),
            skill: meta.map(|meta| meta.skill.clone()),
            agent: meta.map(|meta| meta.agent.clone()),
            tree: meta.and_then(|meta| meta.tree.clone()),
            task_label: meta.and_then(|meta| meta.task_label.clone()),
            started_at: meta.map(|meta| meta.created_at),
            window: meta.map(super::resolve::window_state),
            wake,
            last_status: meta.and(worker.last_status_line()),
            state,
            checkin,
            error,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum WakeState {
    Pending,
    Clear,
}

#[derive(Serialize)]
struct CheckinOutput {
    deadline: DateTime<Utc>,
}

/// Whether this worker is holding a wake the lead has not collected.
///
/// `niles wait` records how far into the status log it has delivered, so everything past that
/// cursor is owed. Only actionable lines count: a trailing `working:` line wakes nobody, and
/// reporting it as pending would send the lead into a `wait` that blocks.
fn has_pending_wake(worker: &WorkerSnapshot) -> bool {
    worker.undelivered().is_some_and(|undelivered| {
        String::from_utf8_lossy(undelivered)
            .lines()
            .any(wake::is_actionable_wake)
    })
}
