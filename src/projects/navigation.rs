use anyhow::Result;
use serde::Serialize;

use super::{rows::State, snapshot, windows::Role};
use crate::worker::WorkerRole;

#[derive(Clone, Serialize)]
pub(crate) struct Project {
    name: String,
    state: ProjectState,
    workers: Vec<Worker>,
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
    role: WorkerRole,
    lost: bool,
}

pub(crate) fn collect() -> Result<Vec<Project>> {
    snapshot::collect()?
        .into_iter()
        .map(|snapshot| {
            let labels = if snapshot.agents.is_some() {
                crate::worker::model_labels(&snapshot.row.entry.path)?
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
    if let Some(agents) = project.agents {
        for window in agents.windows {
            if let Role::Worker(id, role) = window.role {
                workers.push(Worker {
                    model: None,
                    id,
                    role,
                    lost: false,
                });
            }
        }
        workers.extend(agents.lost.into_iter().map(|(id, role)| Worker {
            model: None,
            id,
            role,
            lost: true,
        }));
    }
    Project {
        name: project.row.entry.name.as_str().to_owned(),
        state,
        workers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{
        registry::{Entry, ProjectName},
        rows::Row,
        status::Segment,
        windows::{AgentWindow, SessionAgents},
    };
    use serde_json::json;

    #[test]
    fn projects_expose_all_states_and_only_worker_identity_role_and_loss() {
        for (state, label) in [
            (State::Missing, "missing"),
            (State::NotRunning, "not running"),
            (State::Running, "running"),
            (State::Starting, "starting"),
            (State::Waiting(None), "waiting"),
        ] {
            let window = |role| AgentWindow {
                name: "private-window".into(),
                role,
                segment: Segment {
                    label: "private-label".into(),
                    state: None,
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
                        window(Role::Lead),
                        window(Role::Plain),
                        window(Role::Worker("nav".into(), WorkerRole::Worker)),
                    ],
                    lost: vec![("review".into(), WorkerRole::Reviewer)],
                }),
            });
            assert_eq!(
                serde_json::to_value(project).unwrap(),
                json!({
                    "name": "demo", "state": label, "workers": [
                        {"id": "nav", "role": "worker", "lost": false},
                        {"id": "review", "role": "reviewer", "lost": true},
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
