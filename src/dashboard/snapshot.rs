use std::{cmp::Ordering, collections::BTreeMap};

use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};

use crate::{
    session::{self, SessionMeta},
    store, tmux,
    tmux::TmuxWindowSnapshot,
    wake,
    worker::{self, WorkerDashboardMeta},
};

use super::status::{self, WakeState};

const HOME_WINDOW_NAME: &str = "niles";
const MANAGER_WINDOW_NAME: &str = "niles-manager";
const EMPTY_CELL: &str = "-";

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
    pub(crate) messages: Vec<String>,
}

#[derive(Debug)]
pub(crate) enum TmuxWindows {
    Available(Vec<TmuxWindowSnapshot>),
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub(crate) struct RunStepDashboardMeta {
    pub(crate) run_id: String,
    pub(crate) index: usize,
    pub(crate) label: String,
    pub(crate) agent_family: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) started_at: Option<DateTime<Utc>>,
    pub(crate) window: String,
    pub(crate) status: Utf8PathBuf,
}

impl PaneSummary {
    fn missing() -> Self {
        Self {
            liveness: PaneLiveness::Missing,
            pid: None,
            command: None,
        }
    }

    fn unknown() -> Self {
        Self {
            liveness: PaneLiveness::Unknown,
            pid: None,
            command: None,
        }
    }

    fn from_tmux(window: &TmuxWindowSnapshot) -> Self {
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

pub(crate) fn collect(workspace: &Utf8Path, now: DateTime<Utc>) -> DashboardSnapshot {
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
    let steps = match collect_run_steps(workspace) {
        Ok(steps) => steps,
        Err(err) => {
            messages.push(format!("run-step metadata unavailable: {err:#}"));
            Vec::new()
        }
    };

    assemble_snapshot(
        DashboardSources {
            tmux,
            manager,
            workers,
            steps,
            messages,
        },
        now,
    )
}

pub(crate) fn assemble_snapshot(
    mut sources: DashboardSources,
    now: DateTime<Utc>,
) -> DashboardSnapshot {
    let mut rows = BTreeMap::new();
    insert_home_row(&mut rows);
    let manager_target = sources
        .manager
        .as_ref()
        .and_then(|meta| meta.window.clone());
    if let Some(manager) = sources.manager {
        insert_manager_row(&mut rows, manager, now);
    }
    for worker in sources.workers {
        insert_worker_row(&mut rows, worker, now, &mut sources.messages);
    }
    for step in sources.steps {
        insert_run_step_row(&mut rows, step, now);
    }

    match sources.tmux {
        TmuxWindows::Available(windows) => merge_tmux_windows(&mut rows, windows),
        TmuxWindows::Unavailable(message) => {
            sources.messages.push(message);
            for row in rows.values_mut() {
                row.pane = PaneSummary::unknown();
            }
        }
    }

    let mut rows = rows.into_values().collect::<Vec<_>>();
    rows.sort_by(sort_rows);
    let manager_lifecycle = manager_lifecycle(&rows);
    DashboardSnapshot {
        rows,
        messages: sources.messages,
        manager_target,
        manager_lifecycle,
    }
}

fn insert_home_row(rows: &mut BTreeMap<String, DashboardRow>) {
    rows.insert(
        HOME_WINDOW_NAME.to_owned(),
        DashboardRow {
            window: HOME_WINDOW_NAME.to_owned(),
            role: DashboardRole::Home,
            subject: EMPTY_CELL.to_owned(),
            agent: EMPTY_CELL.to_owned(),
            pane: PaneSummary::missing(),
            wall: EMPTY_CELL.to_owned(),
            wake: WakeState::NotApplicable,
            last_status: EMPTY_CELL.to_owned(),
        },
    );
}

fn insert_manager_row(
    rows: &mut BTreeMap<String, DashboardRow>,
    manager: SessionMeta,
    now: DateTime<Utc>,
) {
    let window = match manager.window.as_deref() {
        Some(target) => window_name_from_target(target),
        None => MANAGER_WINDOW_NAME.to_owned(),
    };
    rows.insert(
        window.clone(),
        DashboardRow {
            window,
            role: DashboardRole::Manager,
            subject: manager.id,
            agent: format_agent_tier(
                &manager.agent,
                manager.agent_family.as_deref(),
                manager.model.as_deref(),
                manager.effort.as_deref(),
            ),
            pane: PaneSummary::missing(),
            wall: format_wall_since(Some(manager.created_at), now),
            wake: WakeState::NotApplicable,
            last_status: EMPTY_CELL.to_owned(),
        },
    );
}

fn insert_worker_row(
    rows: &mut BTreeMap<String, DashboardRow>,
    worker: WorkerDashboardMeta,
    now: DateTime<Utc>,
    messages: &mut Vec<String>,
) {
    let window = window_name_from_target(&worker.window);
    rows.insert(
        window.clone(),
        DashboardRow {
            window,
            role: DashboardRole::Worker,
            subject: worker_subject(&worker),
            agent: format_agent_tier(
                &worker.agent,
                worker.agent_family.as_deref(),
                worker.model.as_deref(),
                worker.effort.as_deref(),
            ),
            pane: PaneSummary::missing(),
            wall: format_wall_since(worker.created_at, now),
            wake: wake_state(&worker.status, &worker.id, messages),
            last_status: last_status_text(&worker.status),
        },
    );
}

fn insert_run_step_row(
    rows: &mut BTreeMap<String, DashboardRow>,
    step: RunStepDashboardMeta,
    now: DateTime<Utc>,
) {
    let window = window_name_from_target(&step.window);
    rows.insert(
        window.clone(),
        DashboardRow {
            window,
            role: DashboardRole::RunStep,
            subject: format!("{}:step-{}", step.run_id, step.index),
            agent: format_agent_tier(
                &step.label,
                step.agent_family.as_deref(),
                step.model.as_deref(),
                step.effort.as_deref(),
            ),
            pane: PaneSummary::missing(),
            wall: format_wall_since(step.started_at, now),
            wake: WakeState::NotApplicable,
            last_status: last_status_text(&step.status),
        },
    );
}

fn merge_tmux_windows(rows: &mut BTreeMap<String, DashboardRow>, windows: Vec<TmuxWindowSnapshot>) {
    for window in windows {
        if !is_managed_window(&window.name) {
            continue;
        }
        let pane = PaneSummary::from_tmux(&window);
        if let Some(row) = rows.get_mut(&window.name) {
            row.pane = pane;
        } else {
            rows.insert(
                window.name.clone(),
                DashboardRow {
                    window: window.name,
                    role: DashboardRole::UnknownNilesWindow,
                    subject: EMPTY_CELL.to_owned(),
                    agent: EMPTY_CELL.to_owned(),
                    pane,
                    wall: EMPTY_CELL.to_owned(),
                    wake: WakeState::NotApplicable,
                    last_status: EMPTY_CELL.to_owned(),
                },
            );
        }
    }
}

fn collect_run_steps(workspace: &Utf8Path) -> anyhow::Result<Vec<RunStepDashboardMeta>> {
    let Some(run_dir) = store::resolve_latest_run_dir_from(workspace)? else {
        return Ok(Vec::new());
    };
    let state = store::read_state(&run_dir)?;
    let status = wake::status_log_path(&run_dir);
    let run_id = state.id.clone();
    let mut steps = Vec::new();
    for step in state.steps {
        let Some(window) = step.window else {
            continue;
        };
        steps.push(RunStepDashboardMeta {
            run_id: run_id.clone(),
            index: step.index,
            label: step.label,
            agent_family: step.agent_family,
            model: step.model,
            effort: step.effort,
            started_at: step.started_at,
            window,
            status: status.clone(),
        });
    }
    steps.sort_by_key(|step| step.index);
    Ok(steps)
}

fn manager_lifecycle(rows: &[DashboardRow]) -> ManagerLifecycle {
    for row in rows {
        if row.role != DashboardRole::Manager {
            continue;
        }
        return match row.pane.liveness {
            PaneLiveness::Live => ManagerLifecycle::Live,
            PaneLiveness::Dead => ManagerLifecycle::Exited,
            PaneLiveness::Missing => ManagerLifecycle::Exited,
            PaneLiveness::Unknown => ManagerLifecycle::Unknown,
        };
    }
    ManagerLifecycle::Unknown
}

fn sort_rows(left: &DashboardRow, right: &DashboardRow) -> Ordering {
    let role_order = role_rank(left.role).cmp(&role_rank(right.role));
    if role_order != Ordering::Equal {
        return role_order;
    }
    left.window.cmp(&right.window)
}

fn role_rank(role: DashboardRole) -> usize {
    match role {
        DashboardRole::Home => 0,
        DashboardRole::Manager => 1,
        DashboardRole::Worker => 2,
        DashboardRole::RunStep => 3,
        DashboardRole::UnknownNilesWindow => 4,
    }
}

fn worker_subject(worker: &WorkerDashboardMeta) -> String {
    match worker.task_label.as_deref() {
        Some(task) => format!("{} ({task})", worker.id),
        None => worker.id.clone(),
    }
}

fn format_agent_tier(
    agent: &str,
    family: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
) -> String {
    if agent.is_empty() && family.is_none() {
        return EMPTY_CELL.to_owned();
    }

    let base = match family {
        Some(family) if model.is_some() || effort.is_some() => family,
        Some(_) => agent,
        None => agent,
    };

    let mut value = base.to_owned();
    if let Some(model) = model {
        value.push('/');
        value.push_str(model);
    }
    if let Some(effort) = effort {
        value.push('/');
        value.push_str(effort);
    }
    value
}

fn format_wall_since(started_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(started_at) = started_at else {
        return EMPTY_CELL.to_owned();
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

fn last_status_text(status_path: &Utf8Path) -> String {
    match status::read_last_actionable_line(status_path) {
        Ok(Some(line)) => line,
        Ok(None) => EMPTY_CELL.to_owned(),
        Err(err) => format!("status-error:{err:#}"),
    }
}

fn wake_state(status_path: &Utf8Path, subject: &str, messages: &mut Vec<String>) -> WakeState {
    match status::read_wake_state(status_path) {
        Ok(state) => state,
        Err(err) => {
            messages.push(format!("wake-state unavailable for {subject}: {err:#}"));
            WakeState::NotApplicable
        }
    }
}

fn window_name_from_target(target: &str) -> String {
    match target.rsplit_once(':') {
        Some((_, window)) if !window.is_empty() => window.to_owned(),
        Some((_, _)) => target.to_owned(),
        None => target.to_owned(),
    }
}

fn is_managed_window(window: &str) -> bool {
    window == HOME_WINDOW_NAME || window.starts_with("niles-")
}
