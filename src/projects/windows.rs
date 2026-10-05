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

pub(super) enum Role {
    Lead,
    Worker(String, worker::WorkerRole),
    Plain,
}

/// One tmux window of a session and the agent it holds. The status bar and the explorer both read
/// windows through this, so they can't disagree about what a window is.
pub(super) struct AgentWindow {
    pub index: u32,
    pub name: String,
    pub role: Role,
    /// Labelled by role alone and never highlighted; each reader adds its own context.
    pub segment: Segment,
}

pub(super) struct SessionAgents {
    pub windows: Vec<AgentWindow>,
    /// Ids of live workers whose recorded window is not in the session.
    pub lost: Vec<(String, worker::WorkerRole)>,
}

pub(super) fn session_agents(
    session: &SessionName,
    project: Option<&Utf8Path>,
    now: DateTime<Utc>,
) -> Result<SessionAgents> {
    let windows = tmux::windows(session)?;
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
        .map(|worker| (worker.id.clone(), worker.role))
        .collect();
    let windows = windows
        .into_iter()
        .map(|window| {
            let (role, model, state, tokens, age) =
                if project.is_some() && window.name == LEAD_WINDOW {
                    let model = lead
                        .as_ref()
                        .map(|lead| lead.model.clone().unwrap_or_else(|| lead.agent.clone()));
                    let usage = lead.as_ref().map(worker::lead_usage).transpose()?.flatten();
                    let alert_when_waiting = !workers.iter().any(|worker| {
                        worker.usage.as_ref().and_then(|usage| usage.state)
                            == Some(SessionState::Working)
                    });
                    let (state, tokens) = usage_fields(usage.as_ref(), alert_when_waiting);
                    (Role::Lead, model, state, tokens, None)
                } else if let Some(worker) = workers
                    .iter()
                    .find(|worker| worker.window == recorded(&window.name))
                {
                    let (state, tokens) = usage_fields(worker.usage.as_ref(), false);
                    (
                        Role::Worker(worker.id.clone(), worker.role),
                        Some(worker.model.clone()),
                        state,
                        tokens,
                        Some(age(now, worker.created_at)),
                    )
                } else {
                    (Role::Plain, None, None, None, None)
                };
            let label = match &role {
                Role::Lead => "lead".to_owned(),
                Role::Worker(id, _) => id.clone(),
                Role::Plain => window.name.clone(),
            };
            Ok(AgentWindow {
                index: window.index,
                name: window.name,
                role,
                segment: Segment {
                    label,
                    model,
                    state,
                    tokens,
                    age,
                    highlighted: false,
                },
            })
        })
        .collect::<Result<_>>()?;
    Ok(SessionAgents { windows, lost })
}

fn usage_fields(
    usage: Option<&Usage>,
    alert_when_waiting: bool,
) -> (Option<theme::State>, Option<u64>) {
    match usage {
        Some(usage) => (
            usage.state.map(|state| match state {
                SessionState::Working => theme::State::Running,
                SessionState::Waiting if alert_when_waiting => theme::State::Waiting,
                SessionState::Waiting => theme::State::Idle,
            }),
            Some(usage.total_tokens()),
        ),
        None => (None, None),
    }
}

pub(super) fn age(now: DateTime<Utc>, since: DateTime<Utc>) -> String {
    let minutes = (now - since).num_minutes().max(0);
    if minutes >= 60 {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}m")
    }
}
