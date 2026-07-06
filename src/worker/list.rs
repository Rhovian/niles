use std::{collections::BTreeMap, fs, io::ErrorKind};

use anyhow::{Context, Error, Result};
use camino::Utf8Path;
use chrono::{DateTime, Utc};

use crate::{
    agent_window,
    store::{self, WorkerLocation},
    usage::{self, UsageAgent, UsageDisplay, UsageRollup, UsageSnapshotInput, UsageSubject},
};

use super::meta::{WorkerMeta, meta_path, metadata_status_path, read_meta_if_exists};

pub(super) const UNLABELED_TASK_LABEL: &str = "-";
const EMPTY_STATUS_PLACEHOLDER: &str = "-";

struct LiveWorker {
    id: String,
    location: WorkerLocation,
    meta: WorkerMeta,
}

pub fn workers(usage: bool) -> Result<()> {
    let workers = live_workers()?;
    if usage {
        return print_workers_usage(workers);
    }

    print_workers_default(workers)
}

fn print_workers_default(workers: Vec<LiveWorker>) -> Result<()> {
    println!(
        "workers[{}]{{id,agent,task,age,window,last_status}}:",
        workers.len()
    );

    let now = Utc::now();
    for worker in workers {
        let task = match worker.meta.task_label.as_deref() {
            Some(task) => task,
            None => UNLABELED_TASK_LABEL,
        };
        let age = worker_age(&worker, now);
        let window = worker_window_state(&worker.meta);
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

fn print_workers_usage(workers: Vec<LiveWorker>) -> Result<()> {
    println!(
        "workers[{}]{{id,agent,task,age,wall,turns,total,input,cache_create,cache_read,cached,output,reasoning,usage}}:",
        workers.len()
    );

    let now = Utc::now();
    let mut rollups = BTreeMap::<String, UsageRollup>::new();
    for worker in workers {
        let task = worker_task_label(&worker).to_owned();
        let age = worker_age(&worker, now);
        let usage = live_worker_usage(&worker, now);
        let rollup = rollups.entry(task.clone()).or_default();
        rollup.add(&usage);
        println!(
            "  {},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            worker.id,
            worker.meta.agent.as_str(),
            task,
            age,
            usage::format_wall(usage.wall_seconds),
            usage::format_optional(usage.turns),
            usage::format_optional(usage.totals.total),
            usage::format_optional(usage.totals.input),
            usage::format_optional(usage.totals.cache_create),
            usage::format_optional(usage.totals.cache_read),
            usage::format_optional(usage.totals.cached),
            usage::format_optional(usage.totals.output),
            usage::format_optional(usage.totals.reasoning),
            usage.status
        );
    }

    println!(
        "task_usage[{}]{{task,workers,total,input,cache_create,cache_read,cached,output,reasoning,wall}}:",
        rollups.len()
    );
    for (task, rollup) in rollups {
        println!(
            "  {},{},{},{},{},{},{},{},{},{}",
            task,
            rollup.available_subjects,
            usage::format_optional(rollup.totals.total),
            usage::format_optional(rollup.totals.input),
            usage::format_optional(rollup.totals.cache_create),
            usage::format_optional(rollup.totals.cache_read),
            usage::format_optional(rollup.totals.cached),
            usage::format_optional(rollup.totals.output),
            usage::format_optional(rollup.totals.reasoning),
            usage::format_rollup_wall(rollup.wall_seconds)
        );
    }

    Ok(())
}

fn live_workers() -> Result<Vec<LiveWorker>> {
    let mut workers = Vec::new();
    for entry in store::resolve_worker_locations()? {
        let meta_path = meta_path(&entry.location.worker_dir);
        if !meta_path.exists() {
            continue;
        }
        let Some(meta) = read_meta_if_exists(&entry.location.worker_dir)? else {
            continue;
        };
        workers.push(LiveWorker {
            id: entry.id,
            location: entry.location,
            meta,
        });
    }
    workers.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(workers)
}

enum WorkerWindowState {
    Live,
    Dead,
    Unknown(Error),
}

impl std::fmt::Display for WorkerWindowState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Live => f.write_str("live"),
            Self::Dead => f.write_str("window-dead"),
            Self::Unknown(err) => write!(f, "window-unknown:{err:#}"),
        }
    }
}

fn worker_window_state(meta: &WorkerMeta) -> WorkerWindowState {
    match agent_window::target_exists(&meta.window) {
        Ok(true) => WorkerWindowState::Live,
        Ok(false) => WorkerWindowState::Dead,
        Err(err) => WorkerWindowState::Unknown(err),
    }
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

fn live_worker_usage(worker: &LiveWorker, now: DateTime<Utc>) -> UsageDisplay {
    let snapshot = usage::compute_usage_snapshot(&UsageSnapshotInput {
        subject: UsageSubject::Worker {
            id: worker.meta.id.clone(),
            task_label: worker.meta.task_label.clone(),
        },
        agent: UsageAgent {
            spec: worker.meta.agent.clone(),
            family: worker.meta.agent_family.clone(),
            model: worker.meta.model.clone(),
            effort: worker.meta.effort.clone(),
        },
        attribution: worker.meta.usage_attribution.clone(),
        started_at: worker_started_at(worker),
        finished_at: now,
        output_path: usage::worker_usage_path(&worker.location.worker_dir),
    });
    UsageDisplay::from_snapshot(&snapshot, true)
}

fn worker_started_at(worker: &LiveWorker) -> Option<DateTime<Utc>> {
    worker
        .meta
        .created_at
        .or_else(|| path_time(&meta_path(&worker.location.worker_dir)))
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
    let status_path = metadata_status_path(&worker.meta, &worker.location.worker_dir);
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
