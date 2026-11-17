use super::status::ForegroundSession;
use super::{registry, rows, windows::LEAD_WINDOW};
use crate::skills::SkillName;
use crate::telemetry::SessionState;
use crate::worker::usage::SessionUsage;
use crate::{session, tmux, worker};
use anyhow::Result;
use serde::{Serialize, Serializer};

#[derive(Serialize)]
pub(crate) struct Project {
    name: String,
    #[serde(serialize_with = "serialize_state")]
    state: rows::State,
    workers: Vec<Worker>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lead_state: Option<AgentState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_error: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Worker {
    Readable {
        model: String,
        id: String,
        skill: SkillName,
        lost: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        state: Option<AgentState>,
    },
    Unreadable {
        id: String,
        error: &'static str,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum AgentState {
    Working,
    Starting,
    Idle,
    Waiting,
}

pub(crate) fn collect() -> Result<Vec<Project>> {
    Ok(records()?.into_iter().map(|(project, _)| project).collect())
}

pub(crate) fn sessions() -> Result<Vec<ForegroundSession>> {
    records()?
        .into_iter()
        .filter_map(|(_, session)| session.transpose())
        .collect()
}

pub(crate) fn push_sessions() -> Result<Vec<ForegroundSession>> {
    Ok(records()?
        .into_iter()
        .flat_map(|(_, session)| session.into_iter().flatten())
        .collect())
}

pub(crate) fn usage(
    workspace: &camino::Utf8Path,
    lead: Option<session::SessionMeta>,
) -> Result<Vec<SessionUsage>> {
    worker::usage::collect_snapshots(
        worker::snapshot::metadata_snapshot(workspace)?
            .into_iter()
            .filter(|worker| worker.meta.is_some()),
        lead,
    )
}

type Record = (Project, Result<Option<ForegroundSession>>);
fn records() -> Result<Vec<Record>> {
    let mut records = Vec::new();
    for entry in registry::entries()? {
        let live = entry.path.is_dir() && entry.has_live_session()?;
        let windows = if live {
            tmux::windows(&entry.name.session()?)?
        } else {
            None
        };
        let lead = session::latest_lead(&entry.path);
        let session_error = lead.is_err().then_some("session metadata is unreadable");
        let activity = match &lead {
            Ok(Some(meta)) if windows.is_some() => {
                Some(worker::lead_session(meta, chrono::Utc::now())?)
            }
            Ok(Some(_) | None) => None,
            Err(_error) => None,
        };
        let mut workers = Vec::new();
        let mut working = false;
        for snapshot in worker::snapshot::metadata_snapshot(&entry.path)? {
            let meta = match snapshot.meta {
                Some(meta) => meta,
                None => {
                    workers.push(Worker::Unreadable {
                        id: snapshot.id,
                        error: "worker metadata is unreadable",
                    });
                    continue;
                }
            };
            if let Some(windows) = &windows {
                let record = worker::usage::meta_session(&meta, chrono::Utc::now())?;
                working |= record.state == SessionState::Working;
                let lost = !windows.iter().any(|window| {
                    meta.window == format!("{}:{}", entry.name.as_str(), window.name)
                });
                workers.push(Worker::Readable {
                    id: snapshot.id,
                    model: match meta.model {
                        Some(model) => model,
                        None => meta.agent,
                    },
                    skill: meta.skill,
                    lost,
                    state: if lost {
                        None
                    } else {
                        Some(agent_state(record.state))
                    },
                });
            }
        }
        workers.sort_by_key(|worker| match worker {
            Worker::Readable { lost, .. } => u8::from(*lost),
            Worker::Unreadable { .. } => 2,
        });
        let lead_activity = activity
            .as_ref()
            .map_or(SessionState::Unknown, |record| record.state);
        let state = if !entry.path.is_dir() {
            rows::State::Missing
        } else if windows.is_none() {
            rows::State::NotRunning
        } else {
            rows::project_state(
                lead_activity,
                activity.as_ref().and_then(|record| record.usage.as_ref()),
                working,
            )
        };
        let lead_state = windows
            .as_ref()
            .and_then(|windows| windows.iter().find(|window| window.name == LEAD_WINDOW))
            .map(|_| match lead_activity {
                SessionState::Working => AgentState::Working,
                SessionState::Starting => AgentState::Starting,
                SessionState::Waiting | SessionState::NeedsAttention | SessionState::Unknown => {
                    AgentState::Waiting
                }
            });
        let waiting = lead_activity.displayed() == SessionState::Waiting;
        let worker_count = workers
            .iter()
            .filter(|worker| matches!(worker, Worker::Readable { lost: false, .. }))
            .count();
        let foreground = if windows.is_some() {
            lead.map(|lead| {
                lead.map(|meta| ForegroundSession {
                    project: entry.name.as_str().to_owned(),
                    meta,
                    waiting,
                    worker_count,
                })
            })
        } else {
            Ok(None)
        };
        let project = Project {
            name: entry.name.as_str().to_owned(),
            state,
            workers,
            lead_state,
            session_error,
        };
        records.push((project, foreground));
    }
    records.sort_by_key(|(project, _)| rows::sort_key(&project.state));
    Ok(records)
}

fn agent_state(state: SessionState) -> AgentState {
    match state {
        SessionState::Working => AgentState::Working,
        SessionState::Starting => AgentState::Starting,
        SessionState::Waiting | SessionState::NeedsAttention | SessionState::Unknown => {
            AgentState::Idle
        }
    }
}

fn serialize_state<S: Serializer>(state: &rows::State, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(match state {
        rows::State::Missing => "missing",
        rows::State::NotRunning => "not running",
        rows::State::Running => "running",
        rows::State::Starting => "starting",
        rows::State::Waiting(_) => "waiting",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn projects_expose_all_states_and_worker_activity_without_private_details() {
        for (state, label) in [
            (rows::State::Missing, "missing"),
            (rows::State::NotRunning, "not running"),
            (rows::State::Running, "running"),
            (rows::State::Starting, "starting"),
            (rows::State::Waiting(None), "waiting"),
            (rows::State::Waiting(Some(chrono::Utc::now())), "waiting"),
        ] {
            let project = Project {
                name: "demo".into(),
                state,
                workers: Vec::new(),
                lead_state: None,
                session_error: None,
            };
            assert_eq!(
                serde_json::to_value(project).unwrap(),
                json!({"name":"demo", "state":label, "workers":[]})
            );
        }
        for (activity, expected) in [
            (SessionState::Working, "working"),
            (SessionState::Starting, "starting"),
            (SessionState::Unknown, "idle"),
            (SessionState::Waiting, "idle"),
            (SessionState::NeedsAttention, "idle"),
        ] {
            assert_eq!(
                serde_json::to_value(agent_state(activity)).unwrap(),
                json!(expected),
                "activity {activity:?}"
            );
        }
    }
}
