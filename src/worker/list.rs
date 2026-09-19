use std::{fs, io::ErrorKind};

use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, Utc};

use crate::{
    store,
};

use super::meta::{WorkerMeta, meta_path, read_meta_if_exists};
use crate::wake;

pub(super) const UNLABELED_TASK_LABEL: &str = "-";
const EMPTY_STATUS_PLACEHOLDER: &str = "-";

struct LiveWorker {
    id: String,
    worker_dir: camino::Utf8PathBuf,
    meta: WorkerMeta,
}

pub fn workers() -> Result<()> {
    let workers = live_workers()?;
    println!(
        "workers[{}]{{id,agent,task,age,window,last_status}}:",
        workers.len()
    );

    let now = Utc::now();
    for worker in workers {
        let task = worker_task_label(&worker);
        let age = worker_age(&worker, now);
        let window = super::resolve::window_state(&worker.meta);
        let status = last_status_line(&worker)?;
        let status = match status {
            Some(status) => status,
            None => EMPTY_STATUS_PLACEHOLDER.to_owned(),
        };
        println!(
            "  {},{},{},{},{},{}",
            worker.id,
            worker.meta.agent.as_str(),
            task,
            age,
            window,
            status
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
        let Some(meta) = read_meta_if_exists(&entry.worker_dir)? else {
            continue;
        };
        workers.push(LiveWorker {
            id: entry.id,
            worker_dir: entry.worker_dir,
            meta,
        });
    }
    workers.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(workers)
}

fn worker_age(worker: &LiveWorker, now: DateTime<Utc>) -> String {
    let started_at = match worker_started_at(worker) {
        Some(started_at) => started_at,
        None => now,
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
    worker
        .meta
        .created_at
        .or_else(|| path_time(&meta_path(&worker.worker_dir)))
}

fn worker_task_label(worker: &LiveWorker) -> &str {
    match worker.meta.task_label.as_deref() {
        Some(task) => task,
        None => UNLABELED_TASK_LABEL,
    }
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
