use crate::workspace_manifest::setup::SetupSelection;
use anyhow::Result;

use crate::{
    agents::{AgentSpec, ModelRoster},
    config::spec::load_project_config_from,
    util::{current_dir_utf8, print_structured_rows},
    workspace_manifest::{self, WorkspaceManifest},
};

pub(crate) fn models(selection: Option<&SetupSelection>) -> Result<()> {
    let root = current_dir_utf8()?;
    let config = load_project_config_from(&root)?;
    let manifest = workspace_manifest::load(&root, selection)?;
    let rows = allowed_rows(&config.models, manifest.as_ref())?;
    print_structured_rows(
        "models",
        [
            "family",
            "model",
            "effort",
            "tier",
            "description",
            "cost",
            "speed",
            "context_window",
        ],
        &rows,
    );
    Ok(())
}

fn allowed_rows(
    models: &ModelRoster,
    manifest: Option<&WorkspaceManifest>,
) -> Result<Vec<[String; 8]>> {
    models.registry_rows(|family, model, effort| {
        let Some(manifest) = manifest else {
            return Ok(true);
        };
        let requested = AgentSpec::from_parts(family, Some(model), effort, models)?;
        manifest.tiers.allows(&requested, models)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::roster::{FIXTURE_REGISTRY, parse};

    #[test]
    fn registry_fixture_models_filters_manifest_union_and_efforts() {
        let models = parse(FIXTURE_REGISTRY).unwrap();
        assert_eq!(allowed_rows(&models, None).unwrap().len(), 4);
        let mut manifest = crate::test_support::manifest();
        manifest.tiers.standard.primary = "codex:fixture:medium".into();
        manifest.tiers.standard.secondaries = vec!["claude:haiku".into()];
        manifest.tiers.frontier = None;
        manifest.tiers.economy = None;
        manifest.tiers.mechanical = None;
        let rows = allowed_rows(&models, Some(&manifest)).unwrap();
        let pairs = rows
            .iter()
            .map(|row| (row[1].as_str(), row[2].as_str()))
            .collect::<Vec<_>>();
        assert_eq!(pairs, [("fixture", "medium"), ("haiku", "")]);
        manifest
            .tiers
            .standard
            .secondaries
            .push("codex:fixture:high".into());
        assert_eq!(allowed_rows(&models, Some(&manifest)).unwrap().len(), 3);
        manifest.tiers.standard.primary = "codex:fixture:invalid".into();
        assert!(allowed_rows(&models, Some(&manifest)).is_err());
    }
}
