use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    store,
    wake::{WakeKind, is_closed_wake, status_log_path},
    worker,
};

use super::WaitResult;

pub(in crate::wait) enum WaitTargets {
    Run {
        target: WaitTarget,
        index: Option<usize>,
    },
    Workers(Vec<WaitTarget>),
}

pub(in crate::wait) enum WaitTarget {
    Run {
        status: Utf8PathBuf,
        run_dir: Utf8PathBuf,
    },
    Worker {
        id: String,
        status: Utf8PathBuf,
        dir: Utf8PathBuf,
    },
}

impl WaitTargets {
    pub(in crate::wait) fn resolve(
        run: Option<String>,
        worker_ids: Vec<String>,
        task: Option<String>,
        index: Option<usize>,
    ) -> Result<Self> {
        if let Some(0) = index {
            bail!("step index must be >= 1");
        }

        let has_workers = !worker_ids.is_empty();
        match (run, has_workers, task) {
            (Some(run), false, None) => {
                let run_dir = store::resolve_run_dir(&run)?;
                Ok(Self::Run {
                    target: WaitTarget::Run {
                        status: status_log_path(&run_dir),
                        run_dir,
                    },
                    index,
                })
            }
            (None, true, None) => {
                reject_index_for_worker_targets(index)?;
                Ok(Self::Workers(resolve_worker_targets(dedup_worker_ids(
                    worker_ids,
                ))?))
            }
            (None, false, Some(label)) => {
                reject_index_for_worker_targets(index)?;
                worker::validate_task_label(&label)?;
                let selection = worker::select_worker_ids_by_task(&label)?;
                Ok(Self::Workers(resolve_task_selection(
                    &label,
                    selection,
                    resolve_worker_target,
                )?))
            }
            (Some(_), true, None) => bail!("use either a run id or --worker <id>, not both"),
            (Some(_), false, Some(_)) => bail!("use either a run id or --task <label>, not both"),
            (Some(_), true, Some(_)) => bail!("use exactly one of a run id, --worker, or --task"),
            (None, true, Some(_)) => bail!("use either --worker <id> or --task <label>, not both"),
            (None, false, None) => {
                bail!("wait requires a run id, --worker <id>, or --task <label>")
            }
        }
    }

    pub(in crate::wait) fn prefix_worker_id(&self) -> bool {
        match self {
            Self::Run { .. } => false,
            Self::Workers(targets) => targets.len() > 1,
        }
    }

    pub(in crate::wait) fn timeout_subject(&self) -> String {
        match self {
            Self::Run { target, .. } => target.status().to_string(),
            Self::Workers(targets) if targets.len() == 1 => targets[0].status().to_string(),
            Self::Workers(_) => "requested workers".to_owned(),
        }
    }
}

impl WaitTarget {
    pub(in crate::wait) fn status(&self) -> &Utf8Path {
        match self {
            WaitTarget::Run { status, .. } | WaitTarget::Worker { status, .. } => status,
        }
    }

    pub(in crate::wait) fn closed_if_missing(&self) -> Option<WaitResult> {
        match self {
            WaitTarget::Run { .. } => None,
            WaitTarget::Worker { id, dir, .. } if !dir.exists() => Some(WaitResult::WorkerClosed {
                id: id.clone(),
                line: crate::wake::line(
                    WakeKind::Closed,
                    &format!("worker '{id}' directory removed"),
                ),
            }),
            WaitTarget::Worker { .. } => None,
        }
    }

    pub(in crate::wait) fn result_for_line(&self, line: String) -> WaitResult {
        match self {
            WaitTarget::Worker { id, .. } if is_closed_wake(&line) => WaitResult::WorkerClosed {
                id: id.clone(),
                line,
            },
            WaitTarget::Worker { .. } => WaitResult::Line(line),
            WaitTarget::Run { .. } => WaitResult::Line(line),
        }
    }

    pub(in crate::wait) fn worker_id(&self) -> Option<&str> {
        match self {
            WaitTarget::Run { .. } => None,
            WaitTarget::Worker { id, .. } => Some(id),
        }
    }
}

fn reject_index_for_worker_targets(index: Option<usize>) -> Result<()> {
    if index.is_some() {
        bail!("--index is only valid with a run target");
    }
    Ok(())
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
    fn worker_targets_reject_index_at_boundary() {
        let err = WaitTargets::resolve(None, vec!["known".to_owned()], None, Some(1))
            .err()
            .unwrap();
        assert!(format!("{err:#}").contains("--index is only valid with a run target"));

        let err = WaitTargets::resolve(None, Vec::new(), Some("issue-48".to_owned()), Some(1))
            .err()
            .unwrap();
        assert!(format!("{err:#}").contains("--index is only valid with a run target"));
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
