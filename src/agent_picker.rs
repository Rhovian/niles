use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use dialoguer::{Input, Select, console::Term};

mod catalog;

use catalog::{
    ModelCatalog, ModelGroup, catalog_source_message, effort_options, insert_model_group,
    model_catalog,
};

use crate::config::{agents, spec::AgentConfig};

pub(crate) fn prompt_agent_value(
    root: &Utf8Path,
    label: &str,
    default: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<String> {
    let term = Term::stderr();
    prompt_agent_value_on_term(&term, root, label, default, agent_configs)
}

fn prompt_agent_value_on_term(
    term: &Term,
    root: &Utf8Path,
    label: &str,
    default: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<String> {
    let choices = agent_choices(default, agent_configs);
    let index = select_choice(term, label, &choices, default_choice_index(&choices))?;
    match choices[index].value.clone() {
        AgentChoice::Preset(agent) => {
            prompt_selected_agent(term, root, &agent, default, agent_configs)
        }
        AgentChoice::Other => prompt_custom_agent_spec(term, default, agent_configs),
    }
}

fn prompt_selected_agent(
    term: &Term,
    root: &Utf8Path,
    agent: &str,
    default: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<String> {
    let spec = agents::parse_spec(agent)?;
    if agents::profile_for(spec.family()).is_none() || spec.model().is_some() {
        return canonical_manifest_agent(agent, agent_configs);
    }

    prompt_builtin_agent(term, root, spec.family(), default, agent_configs)
}

fn prompt_builtin_agent(
    term: &Term,
    root: &Utf8Path,
    family: &str,
    default: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<String> {
    let default_spec = agents::parse_spec(default).ok();
    let default_spec = default_spec.as_ref().filter(|spec| spec.family() == family);
    let catalog = model_catalog(root, family, agent_configs)?;
    if let Some(message) = catalog_source_message(family, &catalog) {
        term.write_line(&message)
            .with_context(|| format!("failed to write {family} catalog source"))?;
    }

    let selection = prompt_model(term, family, default_spec, &catalog, agent_configs)?;
    let model = match selection {
        ModelSelection::Default => return Ok(family.to_owned()),
        ModelSelection::Model(model) => model,
        ModelSelection::Custom(value) => return Ok(value),
    };

    let effort = prompt_effort(term, family, &model, default_spec, &catalog)?;
    let value = agent_value(family, Some(&model), effort.as_deref());
    canonical_manifest_agent(&value, agent_configs)
}

fn prompt_model(
    term: &Term,
    family: &str,
    default_spec: Option<&agents::AgentSpec>,
    catalog: &ModelCatalog,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<ModelSelection> {
    let choices = model_choices(family, default_spec, catalog);
    let default_index = default_choice_index(&choices);
    let index = select_choice(term, &format!("{family} model"), &choices, default_index)?;

    match choices[index].value.clone() {
        ModelChoice::Default => Ok(ModelSelection::Default),
        ModelChoice::Group(group) => {
            choose_model_version(term, family, &group, default_spec).map(ModelSelection::Model)
        }
        ModelChoice::Other => {
            prompt_custom_agent_spec(term, &agent_value(family, None, None), agent_configs)
                .map(ModelSelection::Custom)
        }
    }
}

fn model_choices(
    family: &str,
    default_spec: Option<&agents::AgentSpec>,
    catalog: &ModelCatalog,
) -> Vec<MenuChoice<ModelChoice>> {
    model_choices_from_groups(family, default_spec, catalog.groups.clone())
}

fn model_choices_from_groups(
    family: &str,
    default_spec: Option<&agents::AgentSpec>,
    mut groups: Vec<ModelGroup>,
) -> Vec<MenuChoice<ModelChoice>> {
    if let Some(model) = default_spec.and_then(agents::AgentSpec::model) {
        insert_model_group(&mut groups, family, model);
    }

    let mut choices = vec![MenuChoice {
        label: "default (no model override)".to_owned(),
        value: ModelChoice::Default,
        is_default: default_spec.and_then(agents::AgentSpec::model).is_none(),
    }];
    choices.extend(groups.iter().cloned().map(|group| {
        let is_default = default_spec
            .and_then(agents::AgentSpec::model)
            .is_some_and(|model| group.models.iter().any(|candidate| candidate == model));
        MenuChoice {
            label: group.label.clone(),
            value: ModelChoice::Group(group),
            is_default,
        }
    }));
    choices.push(MenuChoice {
        label: "other...".to_owned(),
        value: ModelChoice::Other,
        is_default: false,
    });
    choices
}

fn choose_model_version(
    term: &Term,
    family: &str,
    group: &ModelGroup,
    default_spec: Option<&agents::AgentSpec>,
) -> Result<String> {
    if group.models.len() == 1 {
        return Ok(group.models[0].clone());
    }

    let default_model = default_spec.and_then(agents::AgentSpec::model);
    let choices = group
        .models
        .iter()
        .map(|model| MenuChoice {
            label: model.clone(),
            value: model.clone(),
            is_default: default_model == Some(model.as_str()),
        })
        .collect::<Vec<_>>();
    let default_index = default_choice_index(&choices);
    let index = select_choice(
        term,
        &format!("{family} {} version", group.label),
        &choices,
        default_index,
    )?;
    Ok(choices[index].value.clone())
}

fn prompt_effort(
    term: &Term,
    family: &str,
    model: &str,
    default_spec: Option<&agents::AgentSpec>,
    catalog: &ModelCatalog,
) -> Result<Option<String>> {
    let efforts = effort_options(family, model, catalog);
    if efforts.is_empty() {
        return Ok(None);
    }

    let default_effort = default_spec
        .filter(|spec| spec.model() == Some(model))
        .and_then(agents::AgentSpec::effort);
    let mut choices = vec![MenuChoice {
        label: "default (no effort override)".to_owned(),
        value: None,
        is_default: default_effort.is_none(),
    }];
    choices.extend(efforts.into_iter().map(|effort| {
        let is_default = default_effort == Some(effort.as_str());
        MenuChoice {
            label: effort.clone(),
            value: Some(effort),
            is_default,
        }
    }));

    let default_index = default_choice_index(&choices);
    let index = select_choice(term, &format!("{family} effort"), &choices, default_index)?;
    Ok(choices[index].value.clone())
}

fn select_choice<T>(
    term: &Term,
    title: &str,
    choices: &[MenuChoice<T>],
    default_index: usize,
) -> Result<usize> {
    let labels = choices
        .iter()
        .map(|choice| choice.label.as_str())
        .collect::<Vec<_>>();
    Select::new()
        .with_prompt(title)
        .items(&labels)
        .default(default_index)
        .interact_on(term)
        .with_context(|| format!("failed to select {title}"))
}

fn prompt_custom_agent_spec(
    term: &Term,
    default: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<String> {
    loop {
        let value = Input::<String>::new()
            .with_prompt("Custom agent spec")
            .default(default.to_owned())
            .interact_on(term)
            .with_context(|| "failed to read custom agent spec")?;
        match canonical_manifest_agent(&value, agent_configs) {
            Ok(value) => return Ok(value),
            Err(err) => term
                .write_line(&format!("Invalid agent spec: {err}"))
                .with_context(|| "failed to write invalid agent spec message")?,
        }
    }
}

fn canonical_manifest_agent(
    value: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<String> {
    let spec = agents::parse_spec(value)?;
    if agents::profile_for(spec.family()).is_some()
        || agent_configs.contains_key(spec.original())
        || agent_configs.contains_key(spec.family())
    {
        return Ok(spec.canonical());
    }

    bail!(
        "unknown agent `{}`; configure it in niles.yaml or choose codex/claude",
        spec.family()
    )
}

fn agent_choices(
    default: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Vec<MenuChoice<AgentChoice>> {
    let default_spec = agents::parse_spec(default).ok();
    let mut seen = BTreeSet::new();
    let mut choices = Vec::new();

    for family in agents::known_agent_ids() {
        push_agent_choice(
            &mut choices,
            &mut seen,
            family,
            default,
            default_spec.as_ref(),
        );
    }
    for agent in agent_configs.keys() {
        push_agent_choice(
            &mut choices,
            &mut seen,
            agent,
            default,
            default_spec.as_ref(),
        );
    }

    choices.push(MenuChoice {
        label: "other...".to_owned(),
        value: AgentChoice::Other,
        is_default: false,
    });
    choices
}

fn push_agent_choice(
    choices: &mut Vec<MenuChoice<AgentChoice>>,
    seen: &mut BTreeSet<String>,
    agent: &str,
    default: &str,
    default_spec: Option<&agents::AgentSpec>,
) {
    if !seen.insert(agent.to_owned()) {
        return;
    }

    let is_default = agent == default
        || default_spec.is_some_and(|spec| {
            spec.model().is_some() && spec.family() == agent && agents::profile_for(agent).is_some()
        });
    choices.push(MenuChoice {
        label: agent.to_owned(),
        value: AgentChoice::Preset(agent.to_owned()),
        is_default,
    });
}

fn agent_value(family: &str, model: Option<&str>, effort: Option<&str>) -> String {
    let mut value = family.to_owned();
    if let Some(model) = model {
        value.push(':');
        value.push_str(model);
    }
    if let Some(effort) = effort {
        value.push(':');
        value.push_str(effort);
    }
    value
}

fn default_choice_index<T>(choices: &[MenuChoice<T>]) -> usize {
    choices
        .iter()
        .position(|choice| choice.is_default)
        .unwrap_or(0)
}

#[derive(Clone)]
struct MenuChoice<T> {
    label: String,
    value: T,
    is_default: bool,
}

#[derive(Clone)]
enum AgentChoice {
    Preset(String),
    Other,
}

#[derive(Clone)]
enum ModelChoice {
    Default,
    Group(ModelGroup),
    Other,
}

enum ModelSelection {
    Default,
    Model(String),
    Custom(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::agents;

    #[test]
    fn default_model_choice_is_no_override_before_catalog_models() {
        let choices = model_choices_from_groups(
            "codex",
            None,
            vec![
                ModelGroup {
                    label: "o3".to_owned(),
                    models: vec!["o3".to_owned()],
                },
                ModelGroup {
                    label: "o3-pro".to_owned(),
                    models: vec!["o3-pro".to_owned()],
                },
            ],
        );

        assert_eq!(choices[0].label, "default (no model override)");
        assert!(choices[0].is_default);
        assert!(matches!(&choices[0].value, ModelChoice::Default));
        assert!(matches!(&choices[1].value, ModelChoice::Group(group) if group.label == "o3"));
        assert!(matches!(&choices[2].value, ModelChoice::Group(group) if group.label == "o3-pro"));
    }

    #[test]
    fn explicit_model_selection_stores_model_with_default_effort() {
        assert_eq!(agent_value("codex", Some("o3-pro"), None), "codex:o3-pro");
        assert_eq!(
            agent_value("codex", Some("o3-pro"), Some("high")),
            "codex:o3-pro:high"
        );
    }

    #[test]
    fn existing_model_default_still_selects_matching_group() {
        let default = agents::parse_spec("codex:o3-pro:xhigh").unwrap();
        let choices = model_choices_from_groups(
            "codex",
            Some(&default),
            vec![
                ModelGroup {
                    label: "o3".to_owned(),
                    models: vec!["o3".to_owned()],
                },
                ModelGroup {
                    label: "o3-pro".to_owned(),
                    models: vec!["o3-pro".to_owned()],
                },
            ],
        );

        assert_eq!(default_choice_index(&choices), 2);
        assert!(matches!(&choices[2].value, ModelChoice::Group(group) if group.label == "o3-pro"));
    }
}
