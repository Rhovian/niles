use std::{collections::BTreeMap, fs};

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ProjectConfig {
    #[serde(default)]
    pub agents: BTreeMap<String, AgentConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    pub binary: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub prompt: PromptMode,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMode {
    #[default]
    Arg,
    Stdin,
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
