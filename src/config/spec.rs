use std::{collections::BTreeMap, fs};

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::{agents::ModelRoster, schema};

#[derive(Debug, Clone)]
pub struct ProjectConfig {
    pub agents: BTreeMap<String, AgentConfig>,
    pub models: ModelRoster,
}

#[derive(Debug, Default, Deserialize)]
struct RawProjectConfig {
    #[serde(default)]
    agents: BTreeMap<String, AgentConfig>,
    #[serde(default)]
    models: crate::agents::roster::RawRoster,
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
    let models = ModelRoster::builtin().context("failed to parse embedded model roster")?;
    for path in [Utf8Path::new("niles.yaml"), Utf8Path::new(".niles.yaml")] {
        let path = root.join(path);
        if path.exists() {
            let body =
                fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
            return schema::parse_yaml(&body)
                .and_then(|raw| project_config(raw, models))
                .with_context(|| format!("failed to parse {path}"));
        }
    }

    project_config(RawProjectConfig::default(), models)
}

fn project_config(raw: RawProjectConfig, models: ModelRoster) -> Result<ProjectConfig> {
    Ok(ProjectConfig {
        agents: raw.agents,
        models: models.with_overrides(raw.models)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_path;

    fn load(body: &str) -> Result<ProjectConfig> {
        let root = temp_test_path("project-models");
        fs::create_dir_all(&root)?;
        fs::write(root.join("niles.yaml"), body)?;
        let result = load_project_config_from(&root);
        fs::remove_dir_all(root)?;
        result
    }

    #[test]
    fn model_overrides_add_replace_and_allow_empty_efforts() {
        let config = load(
            r#"
models:
  codex:
    gpt-5.7: { efforts: [low, med, xhigh] }
    gpt-5.5: { efforts: [high] }
  claude:
    opus: { efforts: [] }
"#,
        )
        .unwrap();

        assert_eq!(
            config.models.supported_efforts("codex", "gpt-5.7").unwrap(),
            ["low", "medium", "xhigh"]
        );
        assert_eq!(
            config.models.supported_efforts("codex", "gpt-5.5").unwrap(),
            ["high"]
        );
        assert!(
            config
                .models
                .supported_efforts("claude", "opus")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn malformed_model_overrides_name_the_file_and_specific_cause() {
        for (body, cause) in [
            (
                "models: { gemini: { pro: { efforts: [] } } }",
                "unknown model family `gemini`",
            ),
            (
                "models: { codex: { 'bad model': { efforts: [] } } }",
                "invalid codex model `bad model`",
            ),
            (
                "models: { codex: { gpt-5.7: { efforts: ['not valid'] } } }",
                "invalid effort `not valid`",
            ),
            (
                "models: { codex: { gpt-5.7: { efforts: [med, medium] } } }",
                "duplicate effort `medium`",
            ),
            (
                "models: { codex: { gpt-5.7: { efforts: [], extra: true } } }",
                "unknown field `extra`",
            ),
            (
                "models: { codex: { GPT-5.7: { efforts: [] }, gpt-5.7: { efforts: [] } } }",
                "duplicate codex model `gpt-5.7` after normalization",
            ),
        ] {
            let error = load(body).unwrap_err();
            let message = format!("{error:#}");
            assert!(message.contains("niles.yaml"), "{message}");
            assert!(message.contains(cause), "expected `{cause}` in `{message}`");
        }
    }
}
