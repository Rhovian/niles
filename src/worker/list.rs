use std::{fs, io::Write};

use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::{
    tmux::TargetState,
    util::current_dir_utf8,
    wait::cursor::{cursor_path, parse_cursor},
    wake, watch,
};

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
    role: Option<super::WorkerRole>,
    agent: Option<String>,
    task_label: Option<String>,
    started_at: Option<DateTime<Utc>>,
    window: Option<TargetState>,
    wake: Option<WakeState>,
    last_status: Option<String>,
    checkin: Option<CheckinOutput>,
    error: Option<String>,
}

impl WorkerOutput {
    fn read(worker: &WorkerSnapshot) -> Result<Self> {
        let meta = worker.meta.as_ref();
        let wake = match meta {
            Some(_) if has_pending_wake(worker)? => Some(WakeState::Pending),
            Some(_) => Some(WakeState::Clear),
            None => None,
        };
        let checkin = watch::Checkin::read(&worker.worker_dir)?.map(|checkin| CheckinOutput {
            deadline: checkin.deadline,
        });

        Ok(Self {
            id: worker.id.clone(),
            role: meta.map(|meta| meta.role),
            agent: meta.map(|meta| meta.agent.clone()),
            task_label: meta.and_then(|meta| meta.task_label.clone()),
            started_at: meta.map(|meta| meta.created_at),
            window: meta.map(super::resolve::window_state),
            wake,
            last_status: meta.and(worker.last_status_line()),
            checkin,
            error: worker.read_error.clone(),
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
fn has_pending_wake(worker: &WorkerSnapshot) -> Result<bool> {
    let delivered = delivered_bytes(&worker.worker_dir)?;
    let Some(undelivered) = worker.undelivered(delivered) else {
        return Ok(false);
    };
    Ok(String::from_utf8_lossy(undelivered)
        .lines()
        .any(wake::is_actionable_wake))
}

/// How far `niles wait` has delivered into this worker's status log. No cursor means no wait has
/// ever consumed a line from it, which is position zero.
fn delivered_bytes(worker_dir: &Utf8Path) -> Result<usize> {
    let path = cursor_path(worker_dir);
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err).with_context(|| format!("failed to read {path}")),
    };
    usize::try_from(parse_cursor(&body, &path)?)
        .with_context(|| format!("wake cursor in {path} exceeds this platform's address space"))
}
