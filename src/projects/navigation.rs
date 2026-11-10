use super::rows::State;
use super::snapshot;
use super::windows::WindowKind;
use crate::skills::SkillName;
use crate::{theme, worker};
use anyhow::Result;
use serde::Serialize;

#[derive(Clone, Serialize)]
pub(crate) struct Project {
    name: String,
    state: ProjectState,
    workers: Vec<Worker>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lead_state: Option<AgentState>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "lowercase")]
enum ProjectState {
    Missing,
    #[serde(rename = "not running")]
    NotRunning,
    Running,
    Starting,
    Waiting,
}

#[derive(Clone, Serialize)]
struct Worker {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    id: String,
    skill: SkillName,
    lost: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<AgentState>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "lowercase")]
enum AgentState {
    Working,
    Starting,
    Idle,
    Waiting,
}

pub(crate) fn collect() -> Result<Vec<Project>> {
    snapshot::collect()?
        .into_iter()
        .map(|snapshot| {
            let labels = if snapshot.agents.is_some() {
                worker::model_labels(&snapshot.row.entry.path)?
            } else {
                Vec::new()
            };
            let mut project = project(snapshot);
            for worker in &mut project.workers {
                worker.model = labels
                    .iter()
                    .find(|(id, _)| id == &worker.id)
                    .map(|(_, model)| model.clone());
            }
            Ok(project)
        })
        .collect()
}

fn project(project: snapshot::Project) -> Project {
    let state = match project.row.state {
        State::Missing => ProjectState::Missing,
        State::NotRunning => ProjectState::NotRunning,
        State::Running => ProjectState::Running,
        State::Starting => ProjectState::Starting,
        State::Waiting(_) => ProjectState::Waiting,
    };
    let mut workers = Vec::new();
    let mut lead_state = None;
    if let Some(agents) = project.agents {
        for window in agents.windows {
            if matches!(window.kind, WindowKind::Lead) {
                lead_state = Some(match window.segment.activity() {
                    theme::State::Running => AgentState::Working,
                    theme::State::Starting => AgentState::Starting,
                    theme::State::Idle | theme::State::Waiting => AgentState::Waiting,
                });
            }
            if let WindowKind::Worker(id, skill) = window.kind {
                workers.push(Worker {
                    model: None,
                    id,
                    skill,
                    lost: false,
                    state: Some(match window.segment.activity() {
                        theme::State::Running => AgentState::Working,
                        theme::State::Starting => AgentState::Starting,
                        theme::State::Idle | theme::State::Waiting => AgentState::Idle,
                    }),
                });
            }
        }
        workers.extend(agents.lost.into_iter().map(|(id, skill)| Worker {
            model: None,
            id,
            skill,
            lost: true,
            state: None,
        }));
    }
    Project {
        name: project.row.entry.name.as_str().to_owned(),
        state,
        workers,
        lead_state,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::registry::{Entry, ProjectName};
    use crate::projects::rows::Row;
    use crate::projects::status::Segment;
    use crate::projects::windows::{AgentWindow, SessionAgents};
    use crate::theme::State as Activity;
    use serde_json::json;

    #[test]
    fn projects_expose_all_states_and_worker_activity_without_private_details() {
        for ((state, label), (activity, expected)) in [
            (State::Missing, "missing"),
            (State::NotRunning, "not running"),
            (State::Running, "running"),
            (State::Starting, "starting"),
            (State::Waiting(None), "waiting"),
        ]
        .into_iter()
        .zip([
            (Some(Activity::Running), "working"),
            (Some(Activity::Starting), "starting"),
            (Some(Activity::Waiting), "idle"),
            (Some(Activity::Idle), "idle"),
            (None, "idle"),
        ]) {
            let window = |kind| AgentWindow {
                name: "private-window".into(),
                kind,
                segment: Segment {
                    label: "private-label".into(),
                    state: activity,
                    tokens: None,
                    age: None,
                },
            };
            let project = project(snapshot::Project {
                row: Row {
                    entry: Entry {
                        name: ProjectName::parse("demo").unwrap(),
                        path: "/private/path".into(),
                    },
                    state,
                },
                agents: Some(SessionAgents {
                    windows: vec![
                        window(WindowKind::Lead),
                        window(WindowKind::Plain),
                        window(WindowKind::Worker("nav".into(), "worker".parse().unwrap())),
                    ],
                    lost: vec![("review".into(), "reviewer".parse().unwrap())],
                }),
            });
            assert_eq!(
                serde_json::to_value(project).unwrap(),
                json!({
                    "name": "demo", "state": label, "lead_state": if expected == "idle" { "waiting" } else { expected }, "workers": [
                        {"id": "nav", "skill": "worker", "lost": false, "state": expected},
                        {"id": "review", "skill": "reviewer", "lost": true},
                    ]
                })
            );
        }
        let project = project(snapshot::Project {
            row: Row {
                entry: Entry {
                    name: ProjectName::parse("offline").unwrap(),
                    path: "/private/path".into(),
                },
                state: State::NotRunning,
            },
            agents: None,
        });
        assert_eq!(
            serde_json::to_value(project).unwrap(),
            json!({"name": "offline", "state": "not running", "workers": []})
        );
    }
}
