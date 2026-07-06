use std::{collections::BTreeMap, fs};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TaskSpec {
    pub goal: String,
    #[serde(default)]
    pub workspace: Option<Utf8PathBuf>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<String, AgentConfig>,
    #[serde(default)]
    pub steps: Vec<TaskStep>,
    #[serde(default)]
    pub commands: BTreeMap<String, CommandConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ProjectConfig {
    #[serde(default)]
    pub workspace: Option<Utf8PathBuf>,
    #[serde(default)]
    pub agents: BTreeMap<String, AgentConfig>,
    #[serde(default)]
    pub commands: BTreeMap<String, CommandConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    pub binary: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub prompt: PromptMode,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TaskStep {
    Agent {
        agent: String,
        task: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
    },
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
    },
    Role {
        role: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum CommandConfig {
    Short(String),
    Full { run: String },
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMode {
    #[default]
    Arg,
    Stdin,
}

pub fn load_task(path: &Utf8Path) -> Result<TaskSpec> {
    let body = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    serde_yaml::from_str(&body).context("failed to parse task YAML")
}

pub fn save_task(path: &Utf8Path, spec: &TaskSpec) -> Result<()> {
    let body = serde_yaml::to_string(spec).context("failed to serialize task YAML")?;
    fs::write(path, body).with_context(|| format!("failed to write {path}"))
}

pub fn load_project_config() -> Result<ProjectConfig> {
    load_project_config_from(Utf8Path::new("."))
}

pub fn load_project_config_from(root: &Utf8Path) -> Result<ProjectConfig> {
    for path in [Utf8Path::new("niles.yaml"), Utf8Path::new(".niles.yaml")] {
        let path = root.join(path);
        if path.exists() {
            let body =
                fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
            return serde_yaml::from_str(&body).with_context(|| format!("failed to parse {path}"));
        }
    }

    Ok(ProjectConfig::default())
}

pub fn apply_project_config(mut spec: TaskSpec, config: ProjectConfig) -> TaskSpec {
    if spec.workspace.is_none() {
        spec.workspace = config.workspace;
    }

    let mut agents = config.agents;
    agents.extend(spec.agents);
    spec.agents = agents;

    let mut commands = config.commands;
    commands.extend(spec.commands);
    spec.commands = commands;

    spec
}

impl CommandConfig {
    pub fn run(&self) -> &str {
        match self {
            CommandConfig::Short(run) => run,
            CommandConfig::Full { run } => run,
        }
    }
}

pub fn summarize_spec(spec: &TaskSpec) -> serde_json::Value {
    let agents = spec
        .agents
        .iter()
        .map(|(id, config)| {
            serde_json::json!({
                "id": id,
                "binary": summarized_agent_binary(id, config),
            })
        })
        .collect::<Vec<_>>();

    let steps = spec
        .steps
        .iter()
        .map(|step| match step {
            TaskStep::Agent { agent, task, role } => {
                serde_json::json!({ "agent": agent, "task": task, "role": role })
            }
            TaskStep::Command { command, role } => {
                serde_json::json!({ "command": command, "role": role })
            }
            TaskStep::Role { role, task } => {
                serde_json::json!({ "role": role, "task": task })
            }
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "workspace": spec.workspace,
        "agents": agents,
        "steps": steps,
        "commands": spec.commands,
    })
}

fn summarized_agent_binary<'a>(id: &'a str, config: &'a AgentConfig) -> &'a str {
    match config.binary.as_deref() {
        Some(binary) => binary,
        None => id,
    }
}
