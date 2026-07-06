use anyhow::{bail, Context, Result};

use crate::config::spec::{CommandConfig, TaskSpec, TaskStep};

use super::{WorkspaceFlowRole, WorkspaceManifest};

pub fn task_uses_role_bindings(spec: &TaskSpec) -> bool {
    spec.steps
        .iter()
        .any(|step| matches!(step, TaskStep::Role { .. }))
}

pub fn resolve_task_roles(mut spec: TaskSpec, manifest: &WorkspaceManifest) -> Result<TaskSpec> {
    let mut validation_command = None;

    for step in &mut spec.steps {
        let TaskStep::Role { role, task } = step else {
            continue;
        };

        let flow_role = WorkspaceFlowRole::parse(role)?;
        let role = role.clone();
        let task = task.clone();
        *step = match flow_role {
            WorkspaceFlowRole::Planner => TaskStep::Agent {
                agent: manifest.planner.clone(),
                task: task.with_context(|| "planner role step requires task text")?,
                role: Some(role),
            },
            WorkspaceFlowRole::Worker => TaskStep::Agent {
                agent: manifest.worker.clone(),
                task: task.with_context(|| "worker role step requires task text")?,
                role: Some(role),
            },
            WorkspaceFlowRole::Reviewer => TaskStep::Agent {
                agent: manifest.reviewer.clone(),
                task: task.with_context(|| "reviewer role step requires task text")?,
                role: Some(role),
            },
            WorkspaceFlowRole::Validation => {
                if task.is_some() {
                    bail!("validation role step must not include task text");
                }
                let command = manifest.validation_command.clone();
                validation_command = Some(command.clone());
                TaskStep::Command {
                    command,
                    role: Some(role),
                }
            }
        };
    }

    if let Some(command) = validation_command {
        spec.commands
            .entry(command.clone())
            .or_insert_with(|| super::default_command_config(&command));
    }

    Ok(spec)
}

pub fn default_command_config(command: &str) -> CommandConfig {
    CommandConfig::Full {
        run: if command == "test" {
            "cargo test".to_owned()
        } else {
            command.to_owned()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{initial_flow, WorkspaceManifest};
    use super::resolve_task_roles;
    use crate::config::spec::{TaskSpec, TaskStep};

    #[test]
    fn resolves_role_steps_from_workspace_manifest() {
        let manifest = WorkspaceManifest {
            manager: "claude".to_owned(),
            planner: "planbot".to_owned(),
            worker: "codebot".to_owned(),
            reviewer: "reviewbot".to_owned(),
            validation_command: "check".to_owned(),
            flow: initial_flow(),
        };
        let spec = TaskSpec {
            goal: "ship".to_owned(),
            workspace: None,
            agents: Default::default(),
            steps: vec![
                TaskStep::Role {
                    role: "planner".to_owned(),
                    task: Some("plan".to_owned()),
                },
                TaskStep::Role {
                    role: "validation".to_owned(),
                    task: None,
                },
            ],
            commands: Default::default(),
        };

        let spec = resolve_task_roles(spec, &manifest).unwrap();

        assert!(matches!(
            &spec.steps[0],
            TaskStep::Agent { agent, role, .. }
                if agent == "planbot" && role.as_deref() == Some("planner")
        ));
        assert!(matches!(
            &spec.steps[1],
            TaskStep::Command { command, role }
                if command == "check" && role.as_deref() == Some("validation")
        ));
        assert!(spec.commands.contains_key("check"));
    }
}
