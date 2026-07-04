use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
};

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    capabilities::{self, ModelProbe},
    config::{
        agents::{self, InvocationDefaults},
        spec::AgentConfig,
    },
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

pub(super) fn write_catalog_source<W: Write>(
    output: &mut W,
    family: &str,
    catalog: &ModelCatalog,
) -> Result<()> {
    match &catalog.source {
        ModelCatalogSource::Fresh(path) => {
            writeln!(output, "{family} model options: probed from {path}")?
        }
        ModelCatalogSource::Static if !catalog.groups.is_empty() => writeln!(
            output,
            "{family} model options: static catalog; run `niles analyze --agent {family}` for probed options."
        )?,
        ModelCatalogSource::Static => {}
    }
    Ok(())
}

pub(super) fn insert_model_group(groups: &mut Vec<ModelGroup>, family: &str, model: &str) {
    if groups
        .iter()
        .any(|group| group.models.iter().any(|candidate| candidate == model))
    {
        return;
    }

    let label = model_group_label(family, model);
    if let Some(group) = groups.iter_mut().find(|group| group.label == label) {
        group.models.push(model.to_owned());
        group.models = ordered_group_models(&group.label, group.models.iter().cloned().collect());
        return;
    }

    groups.push(ModelGroup {
        label,
        models: vec![model.to_owned()],
    });
}

pub(super) fn effort_options(family: &str, model: &str, catalog: &ModelCatalog) -> Vec<String> {
    match &catalog.source {
        ModelCatalogSource::Fresh(_) => {
            fresh_effort_options(family, model, &catalog.accepted_models)
        }
        ModelCatalogSource::Static => agents::supported_efforts(family)
            .iter()
            .map(|effort| (*effort).to_owned())
            .collect(),
    }
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
        let label = model_group_label(family, &probe.model);
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

fn model_group_label(family: &str, model: &str) -> String {
    for alias in agents::default_model_aliases(family) {
        if model == *alias {
            return (*alias).to_owned();
        }
    }

    if family == "claude" && model.starts_with("claude-") {
        for alias in agents::default_model_aliases(family) {
            if model.split('-').any(|part| part == *alias) {
                return (*alias).to_owned();
            }
        }
    }

    model.to_owned()
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
    pub(super) groups: Vec<ModelGroup>,
    source: ModelCatalogSource,
    accepted_models: Vec<ModelProbe>,
}

enum ModelCatalogSource {
    Fresh(Utf8PathBuf),
    Static,
}
