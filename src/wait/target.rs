use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    wake::{WakeKind, is_closed_wake},
    worker,
};

use super::outcome::WaitOutcome;

pub(in crate::wait) enum WaitTargets {
    Workers(Vec<WaitTarget>),
}

pub(in crate::wait) enum WaitTarget {
    Worker {
        id: String,
        status: Utf8PathBuf,
        dir: Utf8PathBuf,
    },
}

impl WaitTargets {
    pub(in crate::wait) fn resolve(worker_ids: Vec<String>, task: Option<String>) -> Result<Self> {
        let has_workers = !worker_ids.is_empty();
        match (has_workers, task) {
            (true, None) => Ok(Self::Workers(resolve_worker_targets(dedup_worker_ids(
                worker_ids,
            ))?)),
            (false, Some(label)) => {
                worker::validate_task_label(&label)?;
                let selection = worker::select_worker_ids_by_task(&label)?;
                Ok(Self::Workers(resolve_task_selection(
                    &label,
                    selection,
                    resolve_worker_target,
                )?))
            }
            (true, Some(_)) => bail!("use either --worker <id> or --task <label>, not both"),
            (false, None) => bail!("wait requires --worker <id> or --task <label>"),
        }
    }

    pub(in crate::wait) fn prefix_worker_id(&self) -> bool {
        match self {
            Self::Workers(targets) => targets.len() > 1,
        }
    }

    pub(in crate::wait) fn timeout_subject(&self) -> String {
        match self {
            Self::Workers(targets) if targets.len() == 1 => targets[0].status().to_string(),
            Self::Workers(_) => "requested workers".to_owned(),
        }
    }
}

impl WaitTarget {
    pub(in crate::wait) fn status(&self) -> &Utf8Path {
        match self {
            WaitTarget::Worker { status, .. } => status,
        }
    }

    pub(in crate::wait) fn closed_if_missing(&self) -> Option<WaitOutcome> {
        match self {
            WaitTarget::Worker { id, status, dir } if !dir.exists() => {
                Some(WaitOutcome::WorkerClosed {
                    worker_id: Some(id.clone()),
                    id: id.clone(),
                    status: status.clone(),
                    line: crate::wake::line(
                        WakeKind::Closed,
                        &format!("worker '{id}' directory removed"),
                    ),
                })
            }
            WaitTarget::Worker { .. } => None,
        }
    }

    pub(in crate::wait) fn result_for_line(&self, line: String) -> WaitOutcome {
        match self {
            WaitTarget::Worker { id, status, .. } if is_closed_wake(&line) => {
                WaitOutcome::WorkerClosed {
                    worker_id: Some(id.clone()),
                    id: id.clone(),
                    status: status.clone(),
                    line,
                }
            }
            WaitTarget::Worker { id, .. } => WaitOutcome::Wake {
                worker_id: Some(id.clone()),
                line,
            },
        }
    }

    pub(in crate::wait) fn worker_id(&self) -> Option<&str> {
        match self {
            WaitTarget::Worker { id, .. } => Some(id),
        }
    }
}

fn dedup_worker_ids(worker_ids: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for id in worker_ids {
        if !deduped.iter().any(|existing| existing == &id) {
            deduped.push(id);
        }
    }
    deduped
}

fn resolve_worker_targets(worker_ids: Vec<String>) -> Result<Vec<WaitTarget>> {
    resolve_worker_targets_with(worker_ids, resolve_worker_target)
}

fn resolve_worker_targets_with(
    worker_ids: Vec<String>,
    mut resolve: impl FnMut(String) -> Result<WaitTarget>,
) -> Result<Vec<WaitTarget>> {
    let mut targets = Vec::with_capacity(worker_ids.len());
    for id in worker_ids {
        targets.push(resolve(id)?);
    }
    if targets.is_empty() {
        bail!("wait requires at least one worker target");
    }
    Ok(targets)
}

fn resolve_task_selection(
    label: &str,
    selection: worker::WorkerCloseSelection,
    resolve: impl FnMut(String) -> Result<WaitTarget>,
) -> Result<Vec<WaitTarget>> {
    if !selection.failures.is_empty() {
        let failures = selection
            .failures
            .into_iter()
            .map(|(id, err)| format!("{id}: {err}"))
            .collect::<Vec<_>>()
            .join("; ");
        bail!("failed to select workers with task label {label}: {failures}");
    }
    if selection.ids.is_empty() {
        bail!("no live workers with task label {label}");
    }
    resolve_worker_targets_with(selection.ids, resolve)
}

fn resolve_worker_target(id: String) -> Result<WaitTarget> {
    let status = worker::status_log_path(&id)?;
    let dir = worker_status_dir(&status)?;
    Ok(WaitTarget::Worker { id, status, dir })
}

fn worker_status_dir(status: &Utf8Path) -> Result<Utf8PathBuf> {
    status
        .parent()
        .map(Utf8Path::to_path_buf)
        .with_context(|| format!("worker status path has no parent directory: {status}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wait::test_support::{TestTempDir, test_worker_target};

    #[test]
    fn task_resolution_selects_labelled_live_workers() {
        let root = TestTempDir::new("task-resolution");
        let selection = worker::WorkerCloseSelection {
            ids: vec!["alpha".to_owned(), "beta".to_owned()],
            failures: Vec::new(),
        };
        let targets = resolve_task_selection("issue-48", selection, |id| {
            Ok(test_worker_target(root.path(), &id, "working: selected\n"))
        })
        .unwrap();
        let ids = targets
            .iter()
            .map(|target| target.worker_id().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["alpha", "beta"]);

        let empty = worker::WorkerCloseSelection {
            ids: Vec::new(),
            failures: Vec::new(),
        };
        let err = resolve_task_selection("missing", empty, |id| {
            Ok(test_worker_target(root.path(), &id, "working: selected\n"))
        })
        .err()
        .unwrap();
        assert!(format!("{err:#}").contains("no live workers with task label missing"));
    }

    #[test]
    fn unknown_worker_id_aborts_batch_resolution() {
        let root = TestTempDir::new("unknown-before-guard");
        let known = test_worker_target(root.path(), "known", "working: known\n");
        let mut known = Some(known);

        let err =
            resolve_worker_targets_with(vec!["known".to_owned(), "missing".to_owned()], |id| {
                match id.as_str() {
                    "known" => Ok(known.take().unwrap()),
                    "missing" => bail!("unknown worker id 'missing'"),
                    other => bail!("unexpected worker id '{other}'"),
                }
            })
            .err()
            .unwrap();

        assert!(format!("{err:#}").contains("unknown worker id 'missing'"));
    }
}
