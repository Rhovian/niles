use std::fs;

use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, Utc};

use crate::{
    util::{current_dir_utf8, print_structured_rows},
    wait::cursor::{cursor_path, parse_cursor},
    wake,
};

use super::{
    meta::meta_path,
    snapshot::{WorkerSnapshot, worker_snapshot},
};

pub(super) const UNLABELED_TASK_LABEL: &str = "-";
const EMPTY_STATUS_PLACEHOLDER: &str = "-";
/// Shown for a worker holding an actionable line the lead has not collected with `niles wait`.
/// Without it a finished worker and one still working render identically.
const PENDING_WAKE: &str = "pending";
const NO_PENDING_WAKE: &str = "-";
/// Shown for a worker whose `meta.json` could not be read; niles cannot reach it, so the lead must
/// remove its directory by hand.
const UNREADABLE_METADATA: &str = "unreadable";
const UNKNOWN_AGE: &str = "?";

/// Renders the workspace's live workers.
///
/// Every fact comes from [`worker_snapshot`], which is also what the watcher reads: a listing that
/// enumerated workers its own way would be a second answer to "which workers are live", and the
/// two would drift.
pub fn workers() -> Result<()> {
    let workers = worker_snapshot(&current_dir_utf8()?)?;
    let now = Utc::now();
    let mut rows = Vec::with_capacity(workers.len());
    for worker in &workers {
        if let Some(error) = &worker.read_error {
            eprintln!(
                "worker {} metadata is unreadable; remove its directory to recover: {error}",
                worker.id
            );
        }
        let agent = match worker.meta.as_ref() {
            Some(meta) => meta.agent.as_str(),
            None => UNREADABLE_METADATA,
        };
        let task = worker_task_label(worker);
        let age = worker_age(worker, now);
        let window = worker_window_state(worker);
        let wake = worker_pending_wake(worker)?;
        let status = worker_last_status(worker);
        rows.push([
            worker.id.clone(),
            agent.to_owned(),
            task.to_owned(),
            age,
            window,
            wake,
            status,
        ]);
    }
    print_structured_rows(
        "workers",
        [
            "id",
            "agent",
            "task",
            "age",
            "window",
            "wake",
            "last_status",
        ],
        &rows,
    );

    Ok(())
}

fn worker_age(worker: &WorkerSnapshot, now: DateTime<Utc>) -> String {
    let started_at = match worker_started_at(worker) {
        Some(started_at) => started_at,
        None => return UNKNOWN_AGE.to_owned(),
    };
    let seconds = now.signed_duration_since(started_at).num_seconds().max(0);

    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 60 * 60 {
        format!("{}m", seconds / 60)
    } else if seconds < 60 * 60 * 24 {
        format!("{}h", seconds / (60 * 60))
    } else {
        format!("{}d", seconds / (60 * 60 * 24))
    }
}

fn worker_started_at(worker: &WorkerSnapshot) -> Option<DateTime<Utc>> {
    let meta = worker.meta.as_ref()?;
    meta.created_at
        .or_else(|| path_time(&meta_path(&worker.worker_dir)))
}

fn worker_task_label(worker: &WorkerSnapshot) -> &str {
    match worker
        .meta
        .as_ref()
        .and_then(|meta| meta.task_label.as_deref())
    {
        Some(task) => task,
        None => UNLABELED_TASK_LABEL,
    }
}

fn worker_window_state(worker: &WorkerSnapshot) -> String {
    match worker.meta.as_ref() {
        Some(meta) => super::resolve::window_state(meta).to_string(),
        None => UNREADABLE_METADATA.to_owned(),
    }
}

fn worker_last_status(worker: &WorkerSnapshot) -> String {
    if worker.meta.is_none() {
        return UNREADABLE_METADATA.to_owned();
    }
    match worker.last_status_line() {
        Some(status) => status,
        None => EMPTY_STATUS_PLACEHOLDER.to_owned(),
    }
}

/// Whether this worker is holding a wake the lead has not collected.
///
/// `niles wait` records how far into the status log it has delivered, so everything past that
/// cursor is owed. Only actionable lines count: a trailing `working:` line wakes nobody, and
/// reporting it as pending would send the lead into a `wait` that blocks.
fn worker_pending_wake(worker: &WorkerSnapshot) -> Result<String> {
    if worker.meta.is_none() {
        return Ok(UNREADABLE_METADATA.to_owned());
    }
    let delivered = delivered_bytes(&worker.worker_dir)?;
    let Some(undelivered) = worker.undelivered(delivered) else {
        return Ok(NO_PENDING_WAKE.to_owned());
    };
    Ok(
        if String::from_utf8_lossy(undelivered)
            .lines()
            .any(wake::is_actionable_wake)
        {
            PENDING_WAKE.to_owned()
        } else {
            NO_PENDING_WAKE.to_owned()
        },
    )
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

#[expect(
    clippy::disallowed_methods,
    reason = "worker age is advisory; unreadable metadata mtime falls back to the current listing time"
)]
fn path_time(path: &Utf8Path) -> Option<DateTime<Utc>> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .map(DateTime::<Utc>::from)
}
