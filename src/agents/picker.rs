use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use dialoguer::{Select, console::Term};

use super::catalog::{ModelCatalog, ModelGroup, model_catalog};

use crate::{agents, config::spec::AgentConfig};

const FIRST_MENU_CHOICE_INDEX: usize = 0;

pub(crate) fn prompt_agent_value(
    root: &Utf8Path,
    label: &str,
    default: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<String> {
    let term = Term::stderr();
    let default_spec = agents::parse_spec(default)?;
    let choices = agent_choices(default, &default_spec, agent_configs);
    let index = select_choice(&term, label, &choices, default_choice_index(&choices))?;
    prompt_selected_agent(
        &term,
        root,
        &choices[index].value,
        &default_spec,
        agent_configs,
    )
}

fn prompt_selected_agent(
    term: &Term,
    root: &Utf8Path,
    agent: &str,
    default_spec: &agents::AgentSpec,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<String> {
    let spec = agents::parse_spec(agent)?;
    if agents::profile_for(spec.family()).is_none() || spec.model().is_some() {
        return agents::canonical_manifest_agent(&spec, agent_configs);
    }

    prompt_builtin_agent(term, root, spec.family(), default_spec, agent_configs)
}

fn prompt_builtin_agent(
    term: &Term,
    root: &Utf8Path,
    family: &str,
    default_spec: &agents::AgentSpec,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<String> {
    let default_spec = match default_spec.family() == family {
        true => Some(default_spec),
        false => None,
    };
    let catalog = model_catalog(root, family, agent_configs)?;
    if let Some(message) = catalog.source_message(family) {
        term.write_line(&message)
            .with_context(|| format!("failed to write {family} catalog source"))?;
    }

    let model = prompt_model(term, family, default_spec, &catalog)?;

    let effort = prompt_effort(term, family, &model, default_spec, &catalog)?;
    let spec = agents::AgentSpec::from_parts(family, Some(&model), effort.as_deref())?;
    agents::canonical_manifest_agent(&spec, agent_configs)
}

fn prompt_model(
    term: &Term,
    family: &str,
    default_spec: Option<&agents::AgentSpec>,
    catalog: &ModelCatalog,
) -> Result<String> {
    let choices = model_choices_from_catalog(family, default_spec, catalog);
    if choices.is_empty() {
        bail!("no {family} model options available");
    }
    let default_index = default_choice_index(&choices);
    let index = select_choice(term, &format!("{family} model"), &choices, default_index)?;
    choose_model_version(term, family, &choices[index].value, default_spec)
}

fn model_choices_from_catalog(
    family: &str,
    default_spec: Option<&agents::AgentSpec>,
    catalog: &ModelCatalog,
) -> Vec<MenuChoice<ModelGroup>> {
    let default_model = default_spec.and_then(agents::AgentSpec::model);
    let groups = catalog.groups_with_default(family, default_model);
    let mut choices = groups
        .iter()
        .cloned()
        .map(|group| {
            let is_default = default_model
                .is_some_and(|model| group.models.iter().any(|candidate| candidate == model));
            MenuChoice {
                label: group.label.clone(),
                value: group,
                is_default,
            }
        })
        .collect::<Vec<_>>();
    if default_model.is_none()
        && let Some(first) = choices.first_mut()
    {
        first.is_default = true;
    }
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
    let efforts = catalog.effort_options(family, model);
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

fn agent_choices(
    default: &str,
    default_spec: &agents::AgentSpec,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Vec<MenuChoice<String>> {
    let mut seen = BTreeSet::new();
    let mut choices = Vec::new();

    for family in agents::known_agent_ids() {
        push_agent_choice(&mut choices, &mut seen, family, default, default_spec);
    }
    for agent in agent_configs.keys() {
        push_agent_choice(&mut choices, &mut seen, agent, default, default_spec);
    }

    choices
}

fn push_agent_choice(
    choices: &mut Vec<MenuChoice<String>>,
    seen: &mut BTreeSet<String>,
    agent: &str,
    default: &str,
    default_spec: &agents::AgentSpec,
) {
    if !seen.insert(agent.to_owned()) {
        return;
    }

    let is_default = agent == default
        || (default_spec.model().is_some()
            && default_spec.family() == agent
            && agents::profile_for(agent).is_some());
    choices.push(MenuChoice {
        label: agent.to_owned(),
        value: agent.to_owned(),
        is_default,
    });
}

fn default_choice_index<T>(choices: &[MenuChoice<T>]) -> usize {
    match choices.iter().position(|choice| choice.is_default) {
        Some(index) => index,
        None => FIRST_MENU_CHOICE_INDEX,
    }
}

#[derive(Clone)]
struct MenuChoice<T> {
    label: String,
    value: T,
    is_default: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    use camino::Utf8Path;

    #[test]
    fn agent_choices_do_not_include_free_text_escape_hatch() {
        let default = agents::parse_spec("codex").unwrap();
        let choices = agent_choices("codex", &default, &BTreeMap::new());
        let labels = choices
            .iter()
            .map(|choice| choice.label.as_str())
            .collect::<Vec<_>>();

        assert_eq!(labels, ["codex", "claude"]);
        assert_eq!(default_choice_index(&choices), 0);
    }

    #[test]
    fn bare_family_defaults_to_first_catalog_model() {
        let catalog = model_catalog(
            Utf8Path::new("/tmp/niles-picker-bare-family"),
            "codex",
            &BTreeMap::new(),
        )
        .unwrap();
        let choices = model_choices_from_catalog("codex", None, &catalog);

        assert_eq!(choices[0].label, "gpt-5.5");
        assert!(choices[0].is_default);
        assert_eq!(choices[0].value.label, "gpt-5.5");
        assert_eq!(choices[1].value.label, "o3");
    }

    #[test]
    fn explicit_model_selection_stores_model_with_default_effort() {
        let spec = agents::AgentSpec::from_parts("codex", Some("o3-pro"), None).unwrap();
        assert_eq!(spec.canonical(), "codex:o3-pro");
        let spec = agents::AgentSpec::from_parts("codex", Some("o3-pro"), Some("high")).unwrap();
        assert_eq!(spec.canonical(), "codex:o3-pro:high");
    }

    #[test]
    fn existing_model_default_still_selects_matching_group() {
        let default = agents::parse_spec("codex:o3-pro:xhigh").unwrap();
        let catalog = model_catalog(
            Utf8Path::new("/tmp/niles-picker-existing-model"),
            "codex",
            &BTreeMap::new(),
        )
        .unwrap();
        let choices = model_choices_from_catalog("codex", Some(&default), &catalog);

        assert_eq!(
            choices[default_choice_index(&choices)].value.label,
            "o3-pro"
        );
    }
}
