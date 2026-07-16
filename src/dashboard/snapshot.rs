use camino::Utf8Path;
use chrono::{DateTime, Utc};

use crate::{
    session::{self, SessionMeta},
    tmux,
    tmux::{SessionName, TmuxWindowSnapshot, WindowTarget},
    worker::{self, WorkerDashboardMeta},
};

use super::{
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
    pub(crate) state: Option<String>,
}

#[derive(Debug)]
pub(crate) struct DashboardSources {
    pub(crate) tmux: TmuxWindows,
    pub(crate) manager: Option<SessionMeta>,
    pub(crate) workers: Vec<WorkerDashboardMeta>,
    pub(crate) messages: Vec<String>,
}

#[derive(Debug)]
pub(crate) enum TmuxWindows {
    Available(Vec<TmuxWindowSnapshot>),
    Unavailable(String),
}

impl PaneSummary {
    pub(super) fn missing() -> Self {
        Self {
            liveness: PaneLiveness::Missing,
            pid: None,
            command: None,
            state: None,
        }
    }

    pub(super) fn unknown() -> Self {
        Self {
            liveness: PaneLiveness::Unknown,
            pid: None,
            command: None,
            state: None,
        }
    }

    pub(super) fn from_state(state: impl ToString) -> Self {
        Self {
            liveness: PaneLiveness::Missing,
            pid: None,
            command: None,
            state: Some(state.to_string()),
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
            state: None,
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
    let tmux = collect_tmux_windows(&manager, &workers, &mut messages);

    assemble_snapshot(
        DashboardSources {
            tmux,
            manager,
            workers,
            messages,
        },
        now,
        usage_cache,
        recompute_usage,
    )
}

fn collect_tmux_windows(
    manager: &Option<SessionMeta>,
    workers: &[WorkerDashboardMeta],
    messages: &mut Vec<String>,
) -> TmuxWindows {
    let mut sessions = std::collections::BTreeSet::<SessionName>::new();
    if let Some(manager) = manager
        && let Some(window) = manager.window.as_deref()
    {
        push_recorded_session(&mut sessions, window, "manager", messages);
    }
    for worker in workers {
        push_recorded_session(&mut sessions, &worker.window, &worker.id, messages);
    }

    let mut windows = Vec::new();
    for session in sessions {
        match tmux::list_windows(&session) {
            Ok(mut listed) => windows.append(&mut listed),
            Err(err) => {
                messages.push(format!("tmux list failure for session {session}: {err:#}"));
            }
        }
    }
    TmuxWindows::Available(windows)
}

fn push_recorded_session(
    sessions: &mut std::collections::BTreeSet<SessionName>,
    target: &str,
    subject: &str,
    messages: &mut Vec<String>,
) {
    match WindowTarget::parse(target) {
        Ok(target) => {
            sessions.insert(target.session().clone());
        }
        Err(err) => {
            messages.push(format!("tmux target unavailable for {subject}: {err:#}"));
        }
    }
}

pub(crate) fn assemble_snapshot(
    mut sources: DashboardSources,
    now: DateTime<Utc>,
    usage_cache: &mut DashboardUsageCache,
    recompute_usage: bool,
) -> DashboardSnapshot {
    let row_assembly = rows::assemble(&mut sources, now, usage_cache, recompute_usage);
    DashboardSnapshot {
        rows: row_assembly.rows,
        messages: sources.messages,
        manager_target: row_assembly.manager_target,
        manager_lifecycle: row_assembly.manager_lifecycle,
    }
}
