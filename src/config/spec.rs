use std::{collections::BTreeMap, fs};

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::schema;

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

/// What a `niles.yaml` agent may say about how it takes its brief.
///
/// Deliberately narrower than `agents::BriefDelivery`: the by-path and system-prompt deliveries
/// need a flag spelling, and an agent configured here has no way to give one.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMode {
    #[default]
    Arg,
    Stdin,
}

pub fn load_project_config_from(root: &Utf8Path) -> Result<ProjectConfig> {
    for path in [Utf8Path::new("niles.yaml"), Utf8Path::new(".niles.yaml")] {
        let path = root.join(path);
        if path.exists() {
            let body =
                fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
            return schema::parse_yaml(&body).with_context(|| format!("failed to parse {path}"));
        }
    }

    Ok(ProjectConfig::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_path;

    #[test]
    fn malformed_project_config_keeps_line_and_column() {
        let root = temp_test_path("malformed-project-config");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("niles.yaml"), "agents:\n  codex: [\n").unwrap();

        let err = load_project_config_from(&root).unwrap_err();
        let chain = err.chain().map(ToString::to_string).collect::<Vec<_>>();

        assert!(chain[0].contains("failed to parse"), "{chain:?}");
        assert!(
            chain
                .iter()
                .any(|message| message.contains("line 2") && message.contains("column")),
            "{chain:?}"
        );
        assert!(
            chain.iter().all(|message| !message.contains("codex: [")),
            "{chain:?}"
        );

        fs::remove_dir_all(root).unwrap();
    }
}
