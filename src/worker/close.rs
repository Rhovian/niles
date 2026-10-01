use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::{
    store,
    tmux::{self, TargetState, WindowTarget},
    util::append_line,
    wake::{self, WakeKind},
};

use super::{
    archive::{archive_worker_dir, capture_final_pane, final_pane_path},
    meta::{meta_path, read_meta_if_exists},
    resolve::{no_live_worker_message, resolve_worker_if_exists},
    validation::validate_task_label,
};

struct WorkerCloseOutcome {
    id: String,
    archive_dir: Utf8PathBuf,
    pane_path: Option<Utf8PathBuf>,
    pane_error: Option<String>,
    window: WindowCloseOutcome,
}

struct KilledWindow {
    target: WindowTarget,
    error: Option<String>,
}

enum WindowCloseOutcome {
    Recorded(KilledWindow),
    Recovered {
        recorded: WindowTarget,
        actual: KilledWindow,
    },
    WindowDead,
    OrphanGone,
    OrphanLegacyCandidate {
        candidate: WindowTarget,
    },
    Unknown {
        error: String,
    },
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
    if selection.ids.is_empty() && selection.unreadable.is_empty() {
        bail!("no live workers with task label {label}");
    }
    close_worker_group(
        format!("--task {label}"),
        selection.ids,
        selection.unreadable,
    )
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
    /// Workers whose `meta.json` could not be read at all. They cannot be matched or skipped by
    /// label, and niles cannot reach them (the tmux target lives only in `meta.json`), so they are
    /// surfaced for manual removal rather than treated as outside the requested task label.
    pub(crate) unreadable: Vec<(String, String)>,
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
            "close {selection} failed for {} worker(s): {}",
            failures.len(),
            failures.join(", ")
        )
    }
}

pub(crate) fn select_worker_ids_by_task(label: &str) -> Result<WorkerCloseSelection> {
    let mut ids = Vec::new();
    let mut unreadable = Vec::new();
    for entry in store::worker_locations(&crate::util::current_dir_utf8()?)? {
        match read_meta_if_exists(&entry.worker_dir) {
            Ok(Some(meta)) if meta.task_label.as_deref() == Some(label) => ids.push(entry.id),
            Ok(_) => {}
            // An unreadable worker carries no label, so it can neither match nor be skipped by
            // `label`; it is kept apart so `wait` can ignore it instead of refusing every label.
            Err(err) => unreadable.push((entry.id, format!("{err:#}"))),
        }
    }
    Ok(WorkerCloseSelection { ids, unreadable })
}

fn print_single_close_outcome(outcome: &WorkerCloseOutcome) {
    if let Some(path) = &outcome.pane_path {
        println!("pane: {path}");
    }
    if let Some(err) = &outcome.pane_error {
        println!("pane not captured for worker {}: {err}", outcome.id);
    }
    print_window_close_detail(outcome);
    println!("archive: {}", outcome.archive_dir);
    println!("closed: {}", outcome.id);
}

fn print_group_close_success(outcome: &WorkerCloseOutcome) {
    println!("  {},closed,{}", outcome.id, outcome.archive_dir);
    if let Some(err) = &outcome.pane_error {
        println!("  {},pane-not-captured,{err}", outcome.id);
    }
    if let Some(err) = outcome.window.kill_error() {
        println!("  {},window-not-closed,{err}", outcome.id);
    }
    if let Some(state) = outcome.window.state() {
        println!("  {},window-state,{state}", outcome.id);
    }
}

fn close_worker_once(id: &str) -> Result<WorkerCloseOutcome> {
    let worker_dir = resolve_worker_if_exists(id)?.with_context(|| no_live_worker_message(id))?;
    let meta = read_meta_if_exists(&worker_dir)?.with_context(|| no_live_worker_message(id))?;
    let status_path = wake::status_log_path(&worker_dir);
    append_closed_sentinel(&status_path, id)?;

    let window = match WindowTarget::parse(&meta.window) {
        Ok(recorded) => match tmux::target_state(&recorded, &meta.project, &meta.id) {
            TargetState::Live | TargetState::PaneExited => {
                WindowCloseOutcome::Recorded(KilledWindow {
                    target: recorded,
                    error: None,
                })
            }
            TargetState::OrphanRecovered { actual } => WindowCloseOutcome::Recovered {
                recorded,
                actual: KilledWindow {
                    target: actual,
                    error: None,
                },
            },
            TargetState::WindowDead => WindowCloseOutcome::WindowDead,
            TargetState::OrphanGone => WindowCloseOutcome::OrphanGone,
            TargetState::OrphanLegacyCandidate { candidate } => {
                WindowCloseOutcome::OrphanLegacyCandidate { candidate }
            }
            TargetState::Unknown { error } => WindowCloseOutcome::Unknown { error },
        },
        Err(err) => WindowCloseOutcome::Unknown {
            error: format!("worker {id} metadata has invalid tmux window target: {err:#}"),
        },
    };

    let (pane_path, pane_error) = match window.kill_target() {
        Some(target) => match capture_final_pane(&worker_dir, target) {
            Ok(path) => (path, None),
            Err(err) => (None, Some(err.to_string())),
        },
        None => (None, None),
    };
    let captured_pane = pane_path.is_some();

    let window = window.kill();

    let finished_at = Utc::now();
    let archive_dir = archive_worker_dir(
        &crate::util::current_dir_utf8()?,
        id,
        &worker_dir,
        finished_at,
    )?;
    let pane_path = captured_pane.then(|| final_pane_path(&archive_dir));
    Ok(WorkerCloseOutcome {
        id: id.to_owned(),
        archive_dir,
        pane_path,
        pane_error,
        window,
    })
}

fn print_window_close_detail(outcome: &WorkerCloseOutcome) {
    if let Some((target, err)) = outcome.window.kill_failure() {
        println!("window {} not closed: {err}", target.render());
        return;
    }

    match &outcome.window {
        WindowCloseOutcome::Recorded(killed) => {
            println!("closed window: {}", killed.target.window());
        }
        WindowCloseOutcome::Recovered { recorded, actual } => {
            println!("closed window: {}", actual.target);
            println!(
                "window state: orphan-recovered:{recorded}->{}",
                actual.target
            );
        }
        WindowCloseOutcome::OrphanLegacyCandidate { candidate } => {
            println!("window state: orphan-legacy-candidate:{candidate}");
            println!("manual_close: tmux kill-window -t {candidate}");
        }
        WindowCloseOutcome::WindowDead => println!("window state: window-dead"),
        WindowCloseOutcome::OrphanGone => println!("window state: orphan-gone"),
        WindowCloseOutcome::Unknown { error } => {
            println!("window state: unknown:{error}");
        }
    }
}

impl WindowCloseOutcome {
    fn kill_target(&self) -> Option<&WindowTarget> {
        match self {
            Self::Recorded(killed) => Some(&killed.target),
            Self::Recovered { actual, .. } => Some(&actual.target),
            Self::WindowDead
            | Self::OrphanGone
            | Self::OrphanLegacyCandidate { .. }
            | Self::Unknown { .. } => None,
        }
    }

    fn kill(mut self) -> Self {
        match &mut self {
            Self::Recorded(killed) => {
                killed.error = tmux::kill_window(&killed.target)
                    .err()
                    .map(|err| err.to_string())
            }
            Self::Recovered { actual, .. } => {
                actual.error = tmux::kill_window(&actual.target)
                    .err()
                    .map(|err| err.to_string())
            }
            Self::WindowDead
            | Self::OrphanGone
            | Self::OrphanLegacyCandidate { .. }
            | Self::Unknown { .. } => {}
        }
        self
    }

    fn kill_error(&self) -> Option<&str> {
        match self {
            Self::Recorded(killed) => killed.error.as_deref(),
            Self::Recovered { actual, .. } => actual.error.as_deref(),
            Self::WindowDead
            | Self::OrphanGone
            | Self::OrphanLegacyCandidate { .. }
            | Self::Unknown { .. } => None,
        }
    }

    fn kill_failure(&self) -> Option<(&WindowTarget, &str)> {
        match self {
            Self::Recorded(killed) => killed.error.as_deref().map(|err| (&killed.target, err)),
            Self::Recovered { actual, .. } => {
                actual.error.as_deref().map(|err| (&actual.target, err))
            }
            Self::WindowDead
            | Self::OrphanGone
            | Self::OrphanLegacyCandidate { .. }
            | Self::Unknown { .. } => None,
        }
    }

    fn state(&self) -> Option<String> {
        match self {
            Self::Recorded(_) => None,
            Self::Recovered { actual, .. } => Some(format!("orphan-recovered:{}", actual.target)),
            Self::WindowDead => Some("window-dead".to_owned()),
            Self::OrphanGone => Some("orphan-gone".to_owned()),
            Self::OrphanLegacyCandidate { candidate } => {
                Some(format!("orphan-legacy-candidate:{candidate}"))
            }
            Self::Unknown { error } => Some(format!("unknown:{error}")),
        }
    }
}

fn close_all_worker_ids() -> Result<Vec<String>> {
    Ok(store::worker_locations(&crate::util::current_dir_utf8()?)?
        .into_iter()
        .filter(|entry| meta_path(&entry.worker_dir).exists())
        .map(|entry| entry.id)
        .collect())
}

fn append_closed_sentinel(path: &Utf8Path, id: &str) -> Result<()> {
    append_line(path, &wake::line(WakeKind::Closed, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_path;
    use std::fs;

    #[test]
    fn closed_sentinel_starts_on_its_own_line() {
        let dir = temp_test_path("worker-sentinel");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("status.log");
        fs::write(&path, "working: close requested").unwrap();

        append_closed_sentinel(&path, "auth-fix").unwrap();

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "working: close requested\nclosed: auth-fix\n"
        );

        fs::remove_dir_all(&dir).unwrap();
    }
}
