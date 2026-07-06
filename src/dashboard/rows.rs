use std::{cmp::Ordering, collections::BTreeMap};

use chrono::{DateTime, Utc};

use crate::{session::SessionMeta, tmux::TmuxWindowSnapshot, worker::WorkerDashboardMeta};

use super::{
    snapshot::{
        DashboardRole, DashboardRow, DashboardSources, ManagerLifecycle, PaneLiveness, PaneSummary,
        RunStepDashboardMeta, TmuxWindows,
    },
    status::{self, WakeState},
    usage_cell::{DashboardUsageCache, UsageCell},
};

const HOME_WINDOW_NAME: &str = "niles";
const MANAGER_WINDOW_NAME: &str = "niles-manager";
const EMPTY_CELL: &str = "-";

pub(super) struct RowAssembly {
    pub(super) rows: Vec<DashboardRow>,
    pub(super) manager_target: Option<String>,
    pub(super) manager_lifecycle: ManagerLifecycle,
}

pub(super) fn assemble(
    sources: &mut DashboardSources,
    now: DateTime<Utc>,
    usage_cache: &mut DashboardUsageCache,
    recompute_usage: bool,
) -> RowAssembly {
    let mut rows = BTreeMap::new();
    insert_home_row(&mut rows);
    let manager_target = sources
        .manager
        .as_ref()
        .and_then(|meta| meta.window.clone());
    if let Some(manager) = sources.manager.take() {
        insert_manager_row(&mut rows, manager, now, usage_cache, recompute_usage);
    }
    for worker in std::mem::take(&mut sources.workers) {
        insert_worker_row(
            &mut rows,
            worker,
            now,
            &mut sources.messages,
            usage_cache,
            recompute_usage,
        );
    }
    for step in std::mem::take(&mut sources.steps) {
        insert_run_step_row(&mut rows, step, now, usage_cache, recompute_usage);
    }

    match std::mem::replace(&mut sources.tmux, TmuxWindows::Unavailable(String::new())) {
        TmuxWindows::Available(windows) => merge_tmux_windows(&mut rows, windows),
        TmuxWindows::Unavailable(message) => {
            if !message.is_empty() {
                sources.messages.push(message);
            }
            for row in rows.values_mut() {
                row.pane = PaneSummary::unknown();
            }
        }
    }

    let mut rows = rows.into_values().collect::<Vec<_>>();
    rows.sort_by(sort_rows);
    let manager_lifecycle = manager_lifecycle(&rows);
    RowAssembly {
        rows,
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
            usage: UsageCell::not_applicable(),
            wake: WakeState::NotApplicable,
            last_status: EMPTY_CELL.to_owned(),
        },
    );
}

fn insert_manager_row(
    rows: &mut BTreeMap<String, DashboardRow>,
    manager: SessionMeta,
    now: DateTime<Utc>,
    usage_cache: &mut DashboardUsageCache,
    recompute_usage: bool,
) {
    let usage = usage_cache.manager_usage(&manager, now, recompute_usage);
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
            usage,
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
    usage_cache: &mut DashboardUsageCache,
    recompute_usage: bool,
) {
    let usage = usage_cache.worker_usage(&worker, now, recompute_usage);
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
            usage,
            wake: wake_state(&worker.status, &worker.id, messages),
            last_status: last_status_text(&worker.status),
        },
    );
}

fn insert_run_step_row(
    rows: &mut BTreeMap<String, DashboardRow>,
    step: RunStepDashboardMeta,
    now: DateTime<Utc>,
    usage_cache: &mut DashboardUsageCache,
    recompute_usage: bool,
) {
    let usage = usage_cache.run_step_usage(&step, now, recompute_usage);
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
            usage,
            wake: WakeState::NotApplicable,
            last_status: last_status_text(&step.status_log),
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
                    usage: UsageCell::not_applicable(),
                    wake: WakeState::NotApplicable,
                    last_status: EMPTY_CELL.to_owned(),
                },
            );
        }
    }
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

fn last_status_text(status_path: &camino::Utf8Path) -> String {
    match status::read_last_actionable_line(status_path) {
        Ok(Some(line)) => line,
        Ok(None) => EMPTY_CELL.to_owned(),
        Err(err) => format!("status-error:{err:#}"),
    }
}

fn wake_state(
    status_path: &camino::Utf8Path,
    subject: &str,
    messages: &mut Vec<String>,
) -> WakeState {
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
