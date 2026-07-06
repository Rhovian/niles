use std::{collections::BTreeMap, fs};

use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};

use crate::{
    session::SessionMeta,
    state::{StepKind, StepStatus},
    usage::{self, UsageAgent, UsageDisplay, UsageDisplayStatus, UsageSnapshotInput, UsageSubject},
    worker::WorkerDashboardMeta,
};

use super::snapshot::RunStepDashboardMeta;

const NOT_APPLICABLE: &str = "n-a";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UsageCell {
    total_tokens: Option<u64>,
    status: UsageCellStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UsageCellStatus {
    Available,
    Pending,
    Unavailable,
    NotApplicable,
}

#[derive(Debug, Default)]
pub(crate) struct DashboardUsageCache {
    cells: BTreeMap<UsageCellKey, UsageCell>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum UsageCellKey {
    Manager(String),
    Worker(String),
    RunStep {
        run_id: String,
        index: usize,
        label: String,
    },
}

impl UsageCell {
    pub(crate) fn not_applicable() -> Self {
        Self {
            total_tokens: None,
            status: UsageCellStatus::NotApplicable,
        }
    }

    pub(crate) fn pending() -> Self {
        Self {
            total_tokens: None,
            status: UsageCellStatus::Pending,
        }
    }

    pub(crate) fn unavailable() -> Self {
        Self {
            total_tokens: None,
            status: UsageCellStatus::Unavailable,
        }
    }

    pub(crate) fn from_display(display: &UsageDisplay) -> Self {
        Self {
            total_tokens: display.totals.total,
            status: match display.status {
                UsageDisplayStatus::Available => UsageCellStatus::Available,
                UsageDisplayStatus::Pending => UsageCellStatus::Pending,
                UsageDisplayStatus::Unavailable => UsageCellStatus::Unavailable,
            },
        }
    }

    pub(crate) fn render(&self) -> String {
        match self.status {
            UsageCellStatus::NotApplicable => NOT_APPLICABLE.to_owned(),
            UsageCellStatus::Available
            | UsageCellStatus::Pending
            | UsageCellStatus::Unavailable => {
                format!(
                    "{} {}",
                    usage::format_optional(self.total_tokens),
                    self.status_label()
                )
            }
        }
    }

    fn status_label(&self) -> &'static str {
        match self.status {
            UsageCellStatus::Available => "available",
            UsageCellStatus::Pending => "pending",
            UsageCellStatus::Unavailable => "unavailable",
            UsageCellStatus::NotApplicable => NOT_APPLICABLE,
        }
    }
}

impl DashboardUsageCache {
    pub(crate) fn manager_usage(
        &mut self,
        manager: &SessionMeta,
        now: DateTime<Utc>,
        recompute: bool,
    ) -> UsageCell {
        let key = UsageCellKey::Manager(manager.id.clone());
        self.cached_or_compute(key, recompute, UsageCell::unavailable, || {
            manager_usage_cell(manager, now)
        })
    }

    pub(crate) fn worker_usage(
        &mut self,
        worker: &WorkerDashboardMeta,
        now: DateTime<Utc>,
        recompute: bool,
    ) -> UsageCell {
        let key = UsageCellKey::Worker(worker.id.clone());
        self.cached_or_compute(key, recompute, UsageCell::pending, || {
            worker_usage_cell(worker, now)
        })
    }

    pub(crate) fn run_step_usage(
        &mut self,
        step: &RunStepDashboardMeta,
        now: DateTime<Utc>,
        recompute: bool,
    ) -> UsageCell {
        let key = UsageCellKey::RunStep {
            run_id: step.run_id.clone(),
            index: step.index,
            label: step.label.clone(),
        };
        self.cached_or_compute(
            key,
            recompute,
            || fallback_step_usage(step),
            || run_step_usage_cell(step, now),
        )
    }

    fn cached_or_compute<Fallback, Compute>(
        &mut self,
        key: UsageCellKey,
        recompute: bool,
        fallback: Fallback,
        compute: Compute,
    ) -> UsageCell
    where
        Fallback: FnOnce() -> UsageCell,
        Compute: FnOnce() -> UsageCell,
    {
        if recompute {
            let cell = compute();
            self.cells.insert(key, cell.clone());
            return cell;
        }

        match self.cells.get(&key) {
            Some(cell) => cell.clone(),
            None => fallback(),
        }
    }
}

fn manager_usage_cell(manager: &SessionMeta, now: DateTime<Utc>) -> UsageCell {
    let snapshot = usage::compute_usage_snapshot(&UsageSnapshotInput {
        subject: UsageSubject::Manager {
            id: manager.id.clone(),
        },
        agent: UsageAgent {
            spec: manager.agent.clone(),
            family: manager.agent_family.clone(),
            model: manager.model.clone(),
            effort: manager.effort.clone(),
        },
        attribution: manager.usage_attribution.clone(),
        started_at: Some(manager.created_at),
        finished_at: now,
        output_path: manager_usage_path(manager),
    });
    UsageCell::from_display(&UsageDisplay::from_snapshot(&snapshot, true))
}

fn worker_usage_cell(worker: &WorkerDashboardMeta, now: DateTime<Utc>) -> UsageCell {
    let snapshot = usage::compute_usage_snapshot(&UsageSnapshotInput {
        subject: UsageSubject::Worker {
            id: worker.id.clone(),
            task_label: worker.task_label.clone(),
        },
        agent: UsageAgent {
            spec: worker.agent.clone(),
            family: worker.agent_family.clone(),
            model: worker.model.clone(),
            effort: worker.effort.clone(),
        },
        attribution: worker.usage_attribution.clone(),
        started_at: worker_started_at(worker),
        finished_at: now,
        output_path: usage::worker_usage_path(&worker.worker_dir),
    });
    UsageCell::from_display(&UsageDisplay::from_snapshot(&snapshot, true))
}

fn run_step_usage_cell(step: &RunStepDashboardMeta, now: DateTime<Utc>) -> UsageCell {
    UsageCell::from_display(&step_usage_display(step, now))
}

fn step_usage_display(step: &RunStepDashboardMeta, now: DateTime<Utc>) -> UsageDisplay {
    let wall_seconds = step_wall_seconds(step, now);
    if matches!(step.step_status, StepStatus::Pending) {
        return UsageDisplay::pending(wall_seconds);
    }

    if let Some(path) = existing_usage_path(step) {
        return match usage::read_usage_snapshot(&path) {
            Ok(snapshot) => UsageDisplay::from_snapshot(&snapshot, false),
            Err(_) => UsageDisplay::unavailable(wall_seconds),
        };
    }

    if step.usage_attribution.is_some() {
        let steps_dir = steps_dir(&step.run_dir);
        let snapshot = usage::compute_usage_snapshot(&UsageSnapshotInput {
            subject: UsageSubject::RunStep {
                run_id: step.run_id.clone(),
                index: step.index,
                role: step.role.clone(),
                label: step.label.clone(),
            },
            agent: UsageAgent {
                spec: step.label.clone(),
                family: step.agent_family.clone(),
                model: step.model.clone(),
                effort: step.effort.clone(),
            },
            attribution: step.usage_attribution.clone(),
            started_at: step.started_at,
            finished_at: now,
            output_path: usage::step_usage_path(&steps_dir, step.index, &step.label),
        });
        return UsageDisplay::from_snapshot(&snapshot, true);
    }

    if matches!(step.step_status, StepStatus::Running) && matches!(step.kind, StepKind::Agent) {
        UsageDisplay::pending(wall_seconds)
    } else {
        UsageDisplay::unavailable(wall_seconds)
    }
}

fn fallback_step_usage(step: &RunStepDashboardMeta) -> UsageCell {
    if matches!(step.step_status, StepStatus::Pending)
        || (matches!(step.step_status, StepStatus::Running) && matches!(step.kind, StepKind::Agent))
    {
        UsageCell::pending()
    } else {
        UsageCell::unavailable()
    }
}

fn existing_usage_path(step: &RunStepDashboardMeta) -> Option<Utf8PathBuf> {
    if let Some(path) = &step.usage
        && path.exists()
    {
        return Some(path.clone());
    }

    let fallback = usage::step_usage_path(&steps_dir(&step.run_dir), step.index, &step.label);
    if fallback.exists() {
        return Some(fallback);
    }
    None
}

fn step_wall_seconds(step: &RunStepDashboardMeta, now: DateTime<Utc>) -> Option<i64> {
    let started_at = step.started_at?;
    let finished_at = match step.finished_at {
        Some(finished_at) => finished_at,
        None => {
            if matches!(step.step_status, StepStatus::Running) {
                now
            } else {
                return None;
            }
        }
    };
    Some((finished_at - started_at).num_seconds().max(0))
}

fn worker_started_at(worker: &WorkerDashboardMeta) -> Option<DateTime<Utc>> {
    match worker.created_at {
        Some(created_at) => Some(created_at),
        None => path_time(&worker.metadata),
    }
}

fn path_time(path: &Utf8Path) -> Option<DateTime<Utc>> {
    match fs::metadata(path).and_then(|metadata| metadata.modified()) {
        Ok(time) => Some(DateTime::<Utc>::from(time)),
        Err(_) => None,
    }
}

fn manager_usage_path(manager: &SessionMeta) -> Utf8PathBuf {
    match manager.brief.parent() {
        Some(session_dir) => session_dir.join("usage.json"),
        None => manager
            .workspace
            .join(".niles")
            .join("sessions")
            .join(&manager.id)
            .join("usage.json"),
    }
}

fn steps_dir(run_dir: &Utf8Path) -> Utf8PathBuf {
    run_dir.join("steps")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use camino::Utf8PathBuf;

    use crate::{usage::UsageAttribution, workspace_manifest::WorkspaceManifest};

    use super::*;

    #[test]
    fn renders_available_pending_and_unavailable_usage_cells() {
        let available = UsageCell {
            total_tokens: Some(12345),
            status: UsageCellStatus::Available,
        };
        let pending = UsageCell::from_display(&UsageDisplay::pending(Some(10)));
        let unavailable = UsageCell::from_display(&UsageDisplay::unavailable(Some(10)));

        assert_eq!(available.render(), "12345 available");
        assert_eq!(pending.render(), "- pending");
        assert_eq!(unavailable.render(), "- unavailable");
    }

    #[test]
    fn manager_usage_compute_does_not_write_usage_snapshot() {
        let root = temp_dir("manager-read-only");
        let session_dir = root.join(".niles/sessions/session");
        fs::create_dir_all(&session_dir).unwrap();
        let brief = session_dir.join("manager.md");
        fs::write(&brief, "brief").unwrap();
        let meta = SessionMeta {
            id: "session".to_owned(),
            agent: WorkspaceManifest::default().manager,
            agent_family: Some("unsupported-family".to_owned()),
            model: None,
            effort: None,
            usage_attribution: Some(UsageAttribution::Unsupported {
                cwd: root.clone(),
                launched_at: "2026-07-06T00:00:00Z".parse().unwrap(),
                niles_prompt_count: Some(1),
            }),
            created_at: "2026-07-06T00:00:00Z".parse().unwrap(),
            workspace: root.clone(),
            brief: brief.clone(),
            window: None,
            launch: None,
        };

        let cell = manager_usage_cell(&meta, "2026-07-06T00:00:05Z".parse().unwrap());

        assert_eq!(cell.render(), "- unavailable");
        assert!(!session_dir.join("usage.json").exists());
        fs::remove_dir_all(root).unwrap();
    }

    fn temp_dir(label: &str) -> Utf8PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
            "niles-dashboard-usage-{label}-{}-{nanos}",
            std::process::id()
        )))
        .unwrap();
        fs::create_dir_all(&path).unwrap();
        path
    }
}
