use std::fs;

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::Deserialize;

use crate::{agents::ModelRoster, store};

#[derive(Debug, Clone)]
pub struct ProjectConfig {
    pub models: ModelRoster,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProjectConfig {
    #[serde(default)]
    models: crate::agents::roster::RawRoster,
}

pub(crate) const PROJECT_CONFIG_FILES: [&str; 2] = ["niles.yaml", ".niles.yaml"];

/// The file of `PROJECT_CONFIG_FILES` that the project's config is read from, if any exists.
pub(crate) fn project_config_file(root: &Utf8Path) -> Result<Option<&'static str>> {
    for file in PROJECT_CONFIG_FILES {
        let path = root.join(file);
        if path
            .try_exists()
            .with_context(|| format!("failed to check {path}"))?
        {
            return Ok(Some(file));
        }
    }
    Ok(None)
}

pub fn load_project_config_from(root: &Utf8Path) -> Result<ProjectConfig> {
    let models = ModelRoster::load().context("failed to load model registry")?;
    let Some(file) = project_config_file(root)? else {
        return project_config(RawProjectConfig::default(), models);
    };
    let path = root.join(file);
    let body = fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
    store::parse_yaml(&body)
        .and_then(|raw| project_config(raw, models))
        .with_context(|| format!("failed to parse {path}"))
}

fn project_config(raw: RawProjectConfig, models: ModelRoster) -> Result<ProjectConfig> {
    Ok(ProjectConfig {
        models: models.with_overrides(raw.models)?,
    })
}

#[cfg(test)]
mod tests {
    use crate::test_support::project_config as load;

    #[test]
    fn executable_overrides_are_rejected() {
        assert!(load("agents: {claude: {binary: other}}\n").is_err());
    }

    #[test]
    fn model_overrides_add_replace_and_allow_empty_efforts() {
        let config = load(
            r#"
models:
  codex:
    gpt-5.7: { efforts: [low, med, xhigh], context_window: 400000 }
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
        assert_eq!(
            config.models.context_window("codex", "gpt-5.7"),
            Some(400_000)
        );
        // An override replaces the whole builtin entry, window included.
        assert_eq!(config.models.context_window("claude", "opus"), None);
        assert_eq!(
            config.models.context_window("claude", "sonnet"),
            Some(1_000_000)
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
