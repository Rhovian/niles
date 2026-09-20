use std::{fs, io::ErrorKind};

use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, Utc};

use crate::store;

use super::meta::{WorkerMeta, meta_path, read_meta_if_exists};
use crate::wake;

pub(super) const UNLABELED_TASK_LABEL: &str = "-";
const EMPTY_STATUS_PLACEHOLDER: &str = "-";
/// Shown for a worker holding an actionable line the lead has not collected with `niles wait`.
/// Without it a worker that finished twenty minutes ago and one still working render identically,
/// which is how a `done:` sat undelivered until a human asked about it.
const PENDING_WAKE: &str = "pending";
const NO_PENDING_WAKE: &str = "-";
/// Shown for a worker whose `meta.json` could not be read; niles cannot reach it, so the lead must
/// remove its directory by hand.
const UNREADABLE_METADATA: &str = "unreadable";
const UNKNOWN_AGE: &str = "?";
const CURSOR_FILE: &str = "status.cursor";

struct LiveWorker {
    id: String,
    worker_dir: camino::Utf8PathBuf,
    meta: Option<WorkerMeta>,
    /// Present when `meta.json` exists but could not be parsed; the worker is otherwise unreachable.
    read_error: Option<String>,
}

pub fn workers() -> Result<()> {
    let workers = live_workers()?;
    println!(
        "workers[{}]{{id,agent,task,age,window,wake,last_status}}:",
        workers.len()
    );

    let now = Utc::now();
    for worker in workers {
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
        let task = worker_task_label(&worker);
        let age = worker_age(&worker, now);
        let window = worker_window_state(&worker);
        let log = status_log(&worker)?;
        let wake = worker_pending_wake(&worker, log.as_deref())?;
        let status = worker_last_status(&worker, log.as_deref());
        println!(
            "  {},{},{},{},{},{},{}",
            worker.id, agent, task, age, window, wake, status
        );
    }

    Ok(())
}

fn live_workers() -> Result<Vec<LiveWorker>> {
    let mut workers = Vec::new();
    for entry in store::resolve_worker_locations()? {
        let meta_path = meta_path(&entry.worker_dir);
        if !meta_path.exists() {
            continue;
        }
        let (meta, read_error) = match read_meta_if_exists(&entry.worker_dir) {
            Ok(Some(meta)) => (Some(meta), None),
            Ok(None) => continue,
            Err(err) => (None, Some(format!("{err:#}"))),
        };
        workers.push(LiveWorker {
            id: entry.id,
            worker_dir: entry.worker_dir,
            meta,
            read_error,
        });
    }
    workers.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(workers)
}

fn worker_age(worker: &LiveWorker, now: DateTime<Utc>) -> String {
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

fn worker_started_at(worker: &LiveWorker) -> Option<DateTime<Utc>> {
    let meta = worker.meta.as_ref()?;
    meta.created_at
        .or_else(|| path_time(&meta_path(&worker.worker_dir)))
}

fn worker_task_label(worker: &LiveWorker) -> &str {
    match worker
        .meta
        .as_ref()
        .and_then(|meta| meta.task_label.as_deref())
    {
        Some(task) => task,
        None => UNLABELED_TASK_LABEL,
    }
}

fn worker_window_state(worker: &LiveWorker) -> String {
    match worker.meta.as_ref() {
        Some(meta) => super::resolve::window_state(meta).to_string(),
        None => UNREADABLE_METADATA.to_owned(),
    }
}

fn worker_last_status(worker: &LiveWorker, log: Option<&str>) -> String {
    if worker.meta.is_none() {
        return UNREADABLE_METADATA.to_owned();
    }
    match log.and_then(last_status_line) {
        Some(status) => status.to_owned(),
        None => EMPTY_STATUS_PLACEHOLDER.to_owned(),
    }
}

/// Whether this worker is holding a wake the lead has not collected.
///
/// `niles wait` records how far into the status log it has delivered, so everything past that
/// cursor is owed. Only actionable lines count: a trailing `working:` line wakes nobody, and
/// reporting it as pending would send the lead into a `wait` that blocks.
fn worker_pending_wake(worker: &LiveWorker, log: Option<&str>) -> Result<String> {
    if worker.meta.is_none() {
        return Ok(UNREADABLE_METADATA.to_owned());
    }
    let Some(log) = log else {
        return Ok(NO_PENDING_WAKE.to_owned());
    };
    let delivered = delivered_bytes(&worker.worker_dir)?;
    // A cursor that cannot slice this log — past its end, or mid-character in one that has been
    // rewritten — describes a log `wait` will rescan from the start, so nothing is delivered.
    let undelivered = match log.get(delivered..) {
        Some(undelivered) => undelivered,
        None => log,
    };
    Ok(if undelivered.lines().any(wake::is_actionable_wake) {
        PENDING_WAKE.to_owned()
    } else {
        NO_PENDING_WAKE.to_owned()
    })
}

/// How far `niles wait` has delivered into this worker's status log. No cursor means no wait has
/// ever consumed a line from it, which is position zero.
fn delivered_bytes(worker_dir: &Utf8Path) -> Result<usize> {
    let path = worker_dir.join(CURSOR_FILE);
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err).with_context(|| format!("failed to read {path}")),
    };
    let body = body.trim();
    if body.is_empty() {
        return Ok(0);
    }
    body.parse()
        .with_context(|| format!("invalid wake cursor in {path}; remove it to resume"))
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

fn status_log(worker: &LiveWorker) -> Result<Option<String>> {
    let status_path = wake::status_log_path(&worker.worker_dir);
    match fs::read_to_string(&status_path) {
        Ok(body) => Ok(Some(body)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed to read {status_path}")),
    }
}

fn last_status_line(log: &str) -> Option<&str> {
    log.lines().rev().find(|line| !line.trim().is_empty())
}
