use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::{
    agent_window, store,
    usage::{self, UsageAgent, UsageSnapshotInput, UsageSubject},
    util::append_line,
    wake::{self, WakeKind},
};

use super::{
    archive::{archive_worker_dir, capture_final_pane, final_pane_path},
    meta::{meta_path, metadata_status_path, read_meta_if_exists, write_meta},
    resolve::{no_live_worker_message, resolve_worker_if_exists},
    validation::{validate_id, validate_task_label},
};

struct WorkerCloseOutcome {
    id: String,
    archive_dir: Utf8PathBuf,
    pane_path: Option<Utf8PathBuf>,
    pane_error: Option<String>,
    window_name: String,
    window_error: Option<String>,
}

/// Tear down spawned workers. The tmux window may already be gone, so window
/// close errors are reported but do not strand metadata.
pub fn worker_close(id: Option<String>, task_label: Option<String>, all: bool) -> Result<()> {
    match (id, task_label, all) {
        (Some(id), None, false) => {
            let outcome = close_worker_once(&id)?;
            print_single_close_outcome(&outcome);
            Ok(())
        }
        (None, Some(label), false) => close_workers_by_task(&label),
        (None, None, true) => close_all_workers(),
        _ => bail!("use a worker id, --task <label>, or --all"),
    }
}

fn close_workers_by_task(label: &str) -> Result<()> {
    validate_task_label(label)?;
    let selection = select_worker_ids_by_task(label)?;
    if selection.ids.is_empty() && selection.failures.is_empty() {
        bail!("no live workers with task label {label}");
    }
    close_worker_group(format!("--task {label}"), selection.ids, selection.failures)
}

fn close_all_workers() -> Result<()> {
    let ids = close_all_worker_ids()?;
    if ids.is_empty() {
        println!("no live workers");
        return Ok(());
    }
    close_worker_group("--all".to_owned(), ids, Vec::new())
}

pub(crate) struct WorkerCloseSelection {
    pub(crate) ids: Vec<String>,
    pub(crate) failures: Vec<(String, String)>,
}

fn close_worker_group(
    selection: String,
    ids: Vec<String>,
    selection_failures: Vec<(String, String)>,
) -> Result<()> {
    println!(
        "workers[{}]{{id,status,archive}}:",
        ids.len() + selection_failures.len()
    );

    let mut failures = Vec::new();
    for (id, err) in selection_failures {
        println!("  {id},failed,-");
        eprintln!("worker {id} close failed: {err}");
        failures.push(id);
    }
    for id in ids {
        match close_worker_once(&id) {
            Ok(outcome) => print_group_close_success(&outcome),
            Err(err) => {
                println!("  {id},failed,-");
                eprintln!("worker {id} close failed: {err:#}");
                failures.push(id);
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        bail!(
            "worker-close {selection} failed for {} worker(s): {}",
            failures.len(),
            failures.join(", ")
        )
    }
}

pub(crate) fn select_worker_ids_by_task(label: &str) -> Result<WorkerCloseSelection> {
    let mut ids = Vec::new();
    let mut failures = Vec::new();
    for entry in store::resolve_workspace_worker_locations()? {
        let meta_path = meta_path(&entry.location.worker_dir);
        if !meta_path.exists() {
            continue;
        }
        match read_meta_if_exists(&entry.location.worker_dir) {
            Ok(Some(meta)) if meta.task_label.as_deref() == Some(label) => ids.push(entry.id),
            Ok(_) => {}
            Err(err) => failures.push((entry.id, format!("{err:#}"))),
        }
    }
    ids.sort();
    ids.dedup();
    failures.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(WorkerCloseSelection { ids, failures })
}

fn print_single_close_outcome(outcome: &WorkerCloseOutcome) {
    if let Some(path) = &outcome.pane_path {
        println!("pane: {path}");
    }
    if let Some(err) = &outcome.pane_error {
        println!("pane not captured for worker {}: {err}", outcome.id);
    }
    if let Some(err) = &outcome.window_error {
        println!("window {} not closed: {err}", outcome.window_name);
    } else {
        println!("closed window: {}", outcome.window_name);
    }
    println!("archive: {}", outcome.archive_dir);
    println!("closed: {}", outcome.id);
}

fn print_group_close_success(outcome: &WorkerCloseOutcome) {
    println!("  {},closed,{}", outcome.id, outcome.archive_dir);
    if let Some(err) = &outcome.pane_error {
        println!("  {},pane-not-captured,{err}", outcome.id);
    }
    if let Some(err) = &outcome.window_error {
        println!("  {},window-not-closed,{err}", outcome.id);
    }
}

fn close_worker_once(id: &str) -> Result<WorkerCloseOutcome> {
    validate_id(id)?;
    let location = resolve_worker_if_exists(id)?.with_context(|| no_live_worker_message(id))?;
    let worker_dir = location.worker_dir.clone();
    let mut meta = read_meta_if_exists(&worker_dir)?.with_context(|| no_live_worker_message(id))?;
    let window_name = agent_window::worker_window_name_from_target(id, &meta.window);
    let status_path = metadata_status_path(&meta, &worker_dir);
    append_closed_sentinel(&status_path, id)?;

    let (pane_path, pane_error) = match capture_final_pane(&worker_dir, Some(&meta), &window_name) {
        Ok(path) => (path, None),
        Err(err) => (None, Some(err.to_string())),
    };
    let captured_pane = pane_path.is_some();

    let window_error = agent_window::close_window(&window_name)
        .err()
        .map(|err| err.to_string());

    let finished_at = Utc::now();
    if let Some(path) = usage::snapshot_usage(UsageSnapshotInput {
        subject: UsageSubject::Worker {
            id: meta.id.clone(),
            task_label: meta.task_label.clone(),
        },
        agent: UsageAgent {
            spec: meta.agent.clone(),
            family: meta.agent_family.clone(),
            model: meta.model.clone(),
            effort: meta.effort.clone(),
        },
        attribution: meta.usage_attribution.clone(),
        started_at: meta.created_at,
        finished_at,
        output_path: usage::worker_usage_path(&worker_dir),
    }) {
        meta.usage = Some(path);
        if let Err(err) = write_meta(&worker_dir, &meta) {
            eprintln!("warning: failed to record usage snapshot path in worker metadata: {err:#}");
        }
    }

    let archive_dir = archive_worker_dir(id, &worker_dir, finished_at)?;
    let pane_path = captured_pane.then(|| final_pane_path(&archive_dir));
    store::unregister_worker_location(
        id,
        Some(&location),
        Some(meta.project.as_path()),
        Some(&worker_dir),
    )?;
    Ok(WorkerCloseOutcome {
        id: id.to_owned(),
        archive_dir,
        pane_path,
        pane_error,
        window_name,
        window_error,
    })
}

fn close_all_worker_ids() -> Result<Vec<String>> {
    let mut ids = store::resolve_workspace_worker_locations()?
        .into_iter()
        .filter(|entry| meta_path(&entry.location.worker_dir).exists())
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

fn append_closed_sentinel(path: &Utf8Path, id: &str) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if !parent.exists() {
        return Ok(());
    }

    append_line(
        path,
        &wake::line(WakeKind::Closed, id),
        |path| format!("failed to open {path} for worker close sentinel"),
        |path| format!("failed to inspect {path} before worker close sentinel"),
        |path| format!("failed to write worker close sentinel to {path}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn closed_sentinel_starts_on_its_own_line() {
        let dir = std::env::temp_dir().join(format!(
            "niles-worker-sentinel-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = Utf8PathBuf::from_path_buf(dir.join("status.log")).unwrap();
        fs::write(&path, "working: close requested").unwrap();

        append_closed_sentinel(&path, "auth-fix").unwrap();

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "working: close requested\nclosed: auth-fix\n"
        );

        fs::remove_dir_all(&dir).unwrap();
    }
}
