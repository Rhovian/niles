use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};

use crate::{
    session::{self, SessionMeta},
    state::{StepKind, StepStatus},
    store, tmux,
    tmux::TmuxWindowSnapshot,
    usage::UsageAttribution,
    wake,
    worker::{self, WorkerDashboardMeta},
};

use super::{
    progress::{self, RunProgress},
    rows,
    status::WakeState,
    usage_cell::{DashboardUsageCache, UsageCell},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DashboardSnapshot {
    pub(crate) rows: Vec<DashboardRow>,
    pub(crate) messages: Vec<String>,
    pub(crate) manager_target: Option<String>,
    pub(crate) manager_lifecycle: ManagerLifecycle,
    pub(crate) progress: Option<RunProgress>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DashboardRow {
    pub(crate) window: String,
    pub(crate) role: DashboardRole,
    pub(crate) subject: String,
    pub(crate) agent: String,
    pub(crate) pane: PaneSummary,
    pub(crate) wall: String,
    pub(crate) usage: UsageCell,
    pub(crate) wake: WakeState,
    pub(crate) last_status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DashboardRole {
    Home,
    Manager,
    Worker,
    RunStep,
    UnknownNilesWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaneLiveness {
    Live,
    Dead,
    Missing,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagerLifecycle {
    Live,
    Exited,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneSummary {
    pub(crate) liveness: PaneLiveness,
    pub(crate) pid: Option<u32>,
    pub(crate) command: Option<String>,
}

#[derive(Debug)]
pub(crate) struct DashboardSources {
    pub(crate) tmux: TmuxWindows,
    pub(crate) manager: Option<SessionMeta>,
    pub(crate) workers: Vec<WorkerDashboardMeta>,
    pub(crate) steps: Vec<RunStepDashboardMeta>,
    pub(crate) progress: Option<RunProgress>,
    pub(crate) messages: Vec<String>,
}

#[derive(Debug)]
pub(crate) enum TmuxWindows {
    Available(Vec<TmuxWindowSnapshot>),
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub(crate) struct RunStepDashboardMeta {
    pub(crate) run_dir: Utf8PathBuf,
    pub(crate) run_id: String,
    pub(crate) index: usize,
    pub(crate) role: Option<String>,
    pub(crate) kind: StepKind,
    pub(crate) label: String,
    pub(crate) agent_family: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) usage_attribution: Option<UsageAttribution>,
    pub(crate) usage: Option<Utf8PathBuf>,
    pub(crate) step_status: StepStatus,
    pub(crate) started_at: Option<DateTime<Utc>>,
    pub(crate) finished_at: Option<DateTime<Utc>>,
    pub(crate) window: String,
    pub(crate) status_log: Utf8PathBuf,
}

impl PaneSummary {
    pub(super) fn missing() -> Self {
        Self {
            liveness: PaneLiveness::Missing,
            pid: None,
            command: None,
        }
    }

    pub(super) fn unknown() -> Self {
        Self {
            liveness: PaneLiveness::Unknown,
            pid: None,
            command: None,
        }
    }

    pub(super) fn from_tmux(window: &TmuxWindowSnapshot) -> Self {
        let liveness = if window.pane_dead {
            PaneLiveness::Dead
        } else {
            PaneLiveness::Live
        };
        Self {
            liveness,
            pid: window.pane_pid,
            command: window.pane_current_command.clone(),
        }
    }
}

pub(crate) fn collect(
    workspace: &Utf8Path,
    now: DateTime<Utc>,
    usage_cache: &mut DashboardUsageCache,
    recompute_usage: bool,
) -> DashboardSnapshot {
    let mut messages = Vec::new();
    let tmux = match tmux::current_or_named_session("niles") {
        Ok(session) => match tmux::list_windows(&session) {
            Ok(windows) => TmuxWindows::Available(windows),
            Err(err) => {
                let message = format!("tmux list failure: {err:#}");
                messages.push(message.clone());
                TmuxWindows::Unavailable(message)
            }
        },
        Err(err) => {
            let message = format!("tmux session failure: {err:#}");
            messages.push(message.clone());
            TmuxWindows::Unavailable(message)
        }
    };
    let manager = match session::read_latest_manager_session(workspace) {
        Ok(manager) => manager,
        Err(err) => {
            messages.push(format!("manager metadata unavailable: {err:#}"));
            None
        }
    };
    let workers = match worker::live_workers_for_dashboard() {
        Ok(workers) => workers,
        Err(err) => {
            messages.push(format!("worker metadata unavailable: {err:#}"));
            Vec::new()
        }
    };
    let run = match collect_run_dashboard(workspace) {
        Ok(run) => run,
        Err(err) => {
            messages.push(format!("run metadata unavailable: {err:#}"));
            RunDashboard::default()
        }
    };

    assemble_snapshot(
        DashboardSources {
            tmux,
            manager,
            workers,
            steps: run.steps,
            progress: run.progress,
            messages,
        },
        now,
        usage_cache,
        recompute_usage,
    )
}

pub(crate) fn assemble_snapshot(
    mut sources: DashboardSources,
    now: DateTime<Utc>,
    usage_cache: &mut DashboardUsageCache,
    recompute_usage: bool,
) -> DashboardSnapshot {
    let progress = sources.progress.take();
    let row_assembly = rows::assemble(&mut sources, now, usage_cache, recompute_usage);
    DashboardSnapshot {
        rows: row_assembly.rows,
        messages: sources.messages,
        manager_target: row_assembly.manager_target,
        manager_lifecycle: row_assembly.manager_lifecycle,
        progress,
    }
}

#[derive(Default)]
struct RunDashboard {
    steps: Vec<RunStepDashboardMeta>,
    progress: Option<RunProgress>,
}

fn collect_run_dashboard(workspace: &Utf8Path) -> anyhow::Result<RunDashboard> {
    let Some(run_dir) = store::resolve_latest_run_dir_from(workspace)? else {
        return Ok(RunDashboard::default());
    };
    let state = store::read_state(&run_dir)?;
    let progress = progress::from_state(&state)?;
    let status_log = wake::status_log_path(&run_dir);
    let run_id = state.id.clone();
    let mut steps = Vec::new();
    for step in state.steps {
        let Some(window) = step.window else {
            continue;
        };
        steps.push(RunStepDashboardMeta {
            run_dir: run_dir.clone(),
            run_id: run_id.clone(),
            index: step.index,
            role: step.role,
            kind: step.kind,
            label: step.label,
            agent_family: step.agent_family,
            model: step.model,
            effort: step.effort,
            usage_attribution: step.usage_attribution,
            usage: step.usage,
            step_status: step.status,
            started_at: step.started_at,
            finished_at: step.finished_at,
            window,
            status_log: status_log.clone(),
        });
    }
    steps.sort_by_key(|step| step.index);
    Ok(RunDashboard { steps, progress })
}
