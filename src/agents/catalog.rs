use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    agents::{
        self, InvocationDefaults,
        capabilities::{self, ModelProbe},
    },
    config::spec::AgentConfig,
};

pub(super) fn model_catalog(
    root: &Utf8Path,
    family: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<ModelCatalog> {
    let config = agents::config_for(agent_configs, family)?;
    if let Some(fresh) =
        capabilities::fresh_accepted_models(root, family, config, InvocationDefaults::Default)?
    {
        return Ok(ModelCatalog {
            source: ModelCatalogSource::Fresh(fresh.path),
            groups: fresh_model_groups(family, &fresh.accepted_models),
            accepted_models: fresh.accepted_models,
        });
    }

    Ok(ModelCatalog {
        source: ModelCatalogSource::Static,
        groups: static_model_groups(family),
        accepted_models: Vec::new(),
    })
}

fn static_model_groups(family: &str) -> Vec<ModelGroup> {
    agents::default_model_aliases(family)
        .iter()
        .map(|model| ModelGroup {
            label: (*model).to_owned(),
            models: vec![(*model).to_owned()],
        })
        .collect()
}

fn fresh_model_groups(family: &str, probes: &[ModelProbe]) -> Vec<ModelGroup> {
    let mut groups = BTreeMap::<String, BTreeSet<String>>::new();
    for probe in probes {
        let label = agents::model_group_label(family, &probe.model);
        groups.entry(label).or_default().insert(probe.model.clone());
    }

    groups
        .into_iter()
        .map(|(label, models)| ModelGroup {
            models: ordered_group_models(&label, models),
            label,
        })
        .collect()
}

fn ordered_group_models(label: &str, models: BTreeSet<String>) -> Vec<String> {
    let mut models = models.into_iter().collect::<Vec<_>>();
    models.sort_by(|left, right| match (left == label, right == label) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => left.cmp(right),
    });
    models
}

fn fresh_effort_options(family: &str, model: &str, probes: &[ModelProbe]) -> Vec<String> {
    let mut accepted = probes
        .iter()
        .filter(|probe| probe.model == model)
        .filter_map(|probe| probe.effort.clone())
        .collect::<BTreeSet<_>>();
    if accepted.is_empty() {
        return agents::supported_efforts(family)
            .iter()
            .map(|effort| (*effort).to_owned())
            .collect();
    }

    let mut efforts = Vec::new();
    for effort in agents::supported_efforts(family) {
        if accepted.remove(*effort) {
            efforts.push((*effort).to_owned());
        }
    }
    efforts.extend(accepted);
    efforts
}

#[derive(Clone)]
pub(super) struct ModelGroup {
    pub(super) label: String,
    pub(super) models: Vec<String>,
}

pub(super) struct ModelCatalog {
    groups: Vec<ModelGroup>,
    source: ModelCatalogSource,
    accepted_models: Vec<ModelProbe>,
}

enum ModelCatalogSource {
    Fresh(Utf8PathBuf),
    Static,
}

impl ModelCatalog {
    pub(super) fn source_message(&self, family: &str) -> Option<String> {
        match &self.source {
            ModelCatalogSource::Fresh(path) => {
                Some(format!("{family} model options: probed from {path}"))
            }
            ModelCatalogSource::Static if !self.groups.is_empty() => Some(format!(
                "{family} model options: static catalog; run `niles analyze --agent {family}` for probed options."
            )),
            ModelCatalogSource::Static => None,
        }
    }

    pub(super) fn groups_with_default(
        &self,
        family: &str,
        default_model: Option<&str>,
    ) -> Vec<ModelGroup> {
        let mut groups = self.groups.clone();
        if let Some(model) = default_model {
            Self::insert_model_group(&mut groups, family, model);
        }
        groups
    }

    pub(super) fn effort_options(&self, family: &str, model: &str) -> Vec<String> {
        match &self.source {
            ModelCatalogSource::Fresh(_) => {
                fresh_effort_options(family, model, &self.accepted_models)
            }
            ModelCatalogSource::Static => agents::supported_efforts(family)
                .iter()
                .map(|effort| (*effort).to_owned())
                .collect(),
        }
    }

    fn insert_model_group(groups: &mut Vec<ModelGroup>, family: &str, model: &str) {
        if groups
            .iter()
            .any(|group| group.models.iter().any(|candidate| candidate == model))
        {
            return;
        }

        let label = agents::model_group_label(family, model);
        if let Some(group) = groups.iter_mut().find(|group| group.label == label) {
            group.models.push(model.to_owned());
            group.models =
                ordered_group_models(&group.label, group.models.iter().cloned().collect());
            return;
        }

        groups.push(ModelGroup {
            label,
            models: vec![model.to_owned()],
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;

    #[test]
    fn fresh_catalog_without_effort_probes_uses_family_efforts() {
        let efforts = fresh_effort_options("codex", "o3", &[model_probe("o3", None)]);

        assert_eq!(
            efforts,
            ["minimal", "low", "medium", "high", "xhigh"].map(str::to_owned)
        );
    }

    #[test]
    fn fresh_catalog_with_effort_probes_filters_to_accepted_efforts() {
        let efforts = fresh_effort_options(
            "codex",
            "o3",
            &[
                model_probe("o3", Some("high")),
                model_probe("o3", Some("low")),
            ],
        );

        assert_eq!(efforts, ["low", "high"].map(str::to_owned));
    }

    fn model_probe(model: &str, effort: Option<&str>) -> ModelProbe {
        ModelProbe {
            model: model.to_owned(),
            effort: effort.map(str::to_owned),
            cli_version: Some("0.142.4".to_owned()),
            probed_at: Utc::now(),
            stdout: String::new(),
            stderr: String::new(),
        }
    }
}
