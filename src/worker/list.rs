use std::{fs, io::ErrorKind};

use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, Utc};

use crate::store;

use super::meta::{WorkerMeta, meta_path, read_meta_if_exists};
use crate::wake;

pub(super) const UNLABELED_TASK_LABEL: &str = "-";
const EMPTY_STATUS_PLACEHOLDER: &str = "-";
/// Shown for a worker whose `meta.json` could not be read; niles cannot reach it, so the lead must
/// remove its directory by hand.
const UNREADABLE_METADATA: &str = "unreadable";
const UNKNOWN_AGE: &str = "?";

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
        "workers[{}]{{id,agent,task,age,window,last_status}}:",
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
        let status = worker_last_status(&worker)?;
        println!(
            "  {},{},{},{},{},{}",
            worker.id, agent, task, age, window, status
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

fn worker_last_status(worker: &LiveWorker) -> Result<String> {
    if worker.meta.is_none() {
        return Ok(UNREADABLE_METADATA.to_owned());
    }
    Ok(match last_status_line(worker)? {
        Some(status) => status,
        None => EMPTY_STATUS_PLACEHOLDER.to_owned(),
    })
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

fn last_status_line(worker: &LiveWorker) -> Result<Option<String>> {
    let status_path = wake::status_log_path(&worker.worker_dir);
    let body = match fs::read_to_string(&status_path) {
        Ok(body) => body,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to read {status_path}")),
    };
    Ok(body
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::to_owned))
}
