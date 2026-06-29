use std::{collections::BTreeMap, fs};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct TaskSpec {
    pub goal: String,
    #[serde(default)]
    pub workspace: Option<Utf8PathBuf>,
    #[serde(default)]
    pub agents: BTreeMap<String, AgentConfig>,
    #[serde(default)]
    pub steps: Vec<TaskStep>,
    #[serde(default)]
    pub commands: BTreeMap<String, CommandConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProjectConfig {
    #[serde(default)]
    pub workspace: Option<Utf8PathBuf>,
    #[serde(default)]
    pub agents: BTreeMap<String, AgentConfig>,
    #[serde(default)]
    pub commands: BTreeMap<String, CommandConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    pub binary: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub prompt: PromptMode,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TaskStep {
    Agent { agent: String, task: String },
    Command { command: String },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum CommandConfig {
    Short(String),
    Full { run: String },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMode {
    Arg,
    Stdin,
}

impl Default for PromptMode {
    fn default() -> Self {
        Self::Arg
    }
}

pub fn load_task(path: &Utf8Path) -> Result<TaskSpec> {
    let body = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    serde_yaml::from_str(&body).context("failed to parse task YAML")
}

pub fn load_project_config() -> Result<ProjectConfig> {
    for path in [Utf8Path::new("niles.yaml"), Utf8Path::new(".niles.yaml")] {
        if path.exists() {
            let body =
                fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
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

pub fn command_config_run(config: &CommandConfig) -> &str {
    match config {
        CommandConfig::Short(run) => run,
        CommandConfig::Full { run } => run,
    }
}

pub fn summarize_spec(spec: &TaskSpec) -> serde_json::Value {
    let agents = spec
        .agents
        .iter()
        .map(|(id, config)| {
            serde_json::json!({
                "id": id,
                "binary": config.binary.as_deref().unwrap_or(id),
            })
        })
        .collect::<Vec<_>>();

    let steps = spec
        .steps
        .iter()
        .map(|step| match step {
            TaskStep::Agent { agent, task } => {
                serde_json::json!({ "agent": agent, "task": task })
            }
            TaskStep::Command { command } => serde_json::json!({ "command": command }),
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "workspace": spec.workspace,
        "agents": agents,
        "steps": steps,
        "commands": spec.commands,
    })
}
