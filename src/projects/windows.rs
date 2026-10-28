use anyhow::Result;
use camino::Utf8Path;
use chrono::{DateTime, Utc};

use super::status::Segment;
use crate::{
    session,
    telemetry::{SessionState, Usage},
    theme,
    tmux::{self, SessionName},
    worker,
};

/// The lead's window in every project session.
pub(super) const LEAD_WINDOW: &str = "niles";

pub(super) enum WindowKind {
    Lead,
    Worker(String, crate::skills::SkillName),
    Plain,
}

/// One tmux window of a session and the agent it holds. The status bar and the explorer both read
/// windows through this, so they can't disagree about what a window is.
pub(super) struct AgentWindow {
    pub name: String,
    pub kind: WindowKind,
    /// Labelled by skill alone; each reader adds its own context.
    pub segment: Segment,
}

pub(super) struct SessionAgents {
    pub windows: Vec<AgentWindow>,
    /// Ids of live workers whose recorded window is not in the session.
    pub lost: Vec<(String, crate::skills::SkillName)>,
}

pub(super) fn session_agents(
    session: &SessionName,
    project: Option<&Utf8Path>,
    now: DateTime<Utc>,
) -> Result<Option<SessionAgents>> {
    let Some(windows) = tmux::windows(session)? else {
        return Ok(None);
    };
    let (lead, workers) = match project {
        Some(path) => (session::latest_lead(path)?, worker::status_workers(path)?),
        None => (None, Vec::new()),
    };
    let recorded = |name: &str| format!("{session}:{name}");
    let lost = workers
        .iter()
        .filter(|worker| {
            !windows
                .iter()
                .any(|window| worker.window == recorded(&window.name))
        })
        .map(|worker| (worker.id.clone(), worker.skill.clone()))
        .collect();
    let windows = windows
        .into_iter()
        .map(|window| {
            let (kind, state, tokens, age) = if project.is_some() && window.name == LEAD_WINDOW {
                let record = lead
                    .as_ref()
                    .map(|meta| worker::lead_session(meta, now))
                    .transpose()?;
                let state = record
                    .as_ref()
                    .map_or(SessionState::Unknown, |record| record.state);
                let usage = record.and_then(|record| record.usage);
                let (state, tokens) = usage_fields(state, usage.as_ref());
                let runtime = lead.as_ref().map(|lead| age(now, lead.created_at));
                (WindowKind::Lead, state, tokens, runtime)
            } else if let Some(worker) = workers
                .iter()
                .find(|worker| worker.window == recorded(&window.name))
            {
                let (state, tokens) = usage_fields(worker.state, worker.usage.as_ref());
                (
                    WindowKind::Worker(worker.id.clone(), worker.skill.clone()),
                    state,
                    tokens,
                    Some(age(now, worker.created_at)),
                )
            } else {
                (WindowKind::Plain, None, None, None)
            };
            let worker_id = match &kind {
                WindowKind::Worker(id, _) => Some(id.as_str()),
                WindowKind::Lead | WindowKind::Plain => None,
            };
            let label = window_label(&window.name, project.is_some(), worker_id);
            Ok(AgentWindow {
                name: window.name,
                kind,
                segment: Segment {
                    label,
                    state,
                    tokens,
                    age,
                },
            })
        })
        .collect::<Result<_>>()?;
    Ok(Some(SessionAgents { windows, lost }))
}

pub(super) fn labels(
    session: &SessionName,
    project: Option<&Utf8Path>,
) -> Result<Vec<(u32, String)>> {
    let Some(windows) = tmux::windows(session)? else {
        return Ok(Vec::new());
    };
    let workers = match project {
        Some(path) => worker::window_labels(path)?,
        None => Vec::new(),
    };
    Ok(windows
        .into_iter()
        .map(|window| {
            let recorded = format!("{session}:{}", window.name);
            let worker = workers.iter().find(|(_, target)| *target == recorded);
            let label = window_label(
                &window.name,
                project.is_some(),
                worker.map(|(id, _)| id.as_str()),
            );
            (window.index, label)
        })
        .collect())
}

fn window_label(name: &str, project: bool, worker: Option<&str>) -> String {
    match (project && name == LEAD_WINDOW, worker) {
        (true, _) => "start",
        (false, Some(id)) => id,
        (false, None) => name,
    }
    .to_owned()
}

fn usage_fields(state: SessionState, usage: Option<&Usage>) -> (Option<theme::State>, Option<u64>) {
    let state = match state {
        SessionState::Working => theme::State::Running,
        SessionState::Starting => theme::State::Starting,
        SessionState::Waiting | SessionState::Unknown => theme::State::Idle,
    };
    (Some(state), usage.map(Usage::total_tokens))
}

pub(super) fn age(now: DateTime<Utc>, since: DateTime<Utc>) -> String {
    let minutes = (now - since).num_minutes().max(0);
    if minutes >= 60 {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}m")
    }
}
