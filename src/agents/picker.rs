use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use dialoguer::{Select, console::Term};

use crate::{agents, config::spec::AgentConfig};

const FIRST_MENU_CHOICE_INDEX: usize = 0;

pub(crate) fn prompt_agent_value(
    label: &str,
    default: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
    models: &agents::ModelRoster,
) -> Result<String> {
    let term = Term::stderr();
    let default_spec = agents::parse_spec(default, models)?;
    let choices = agent_choices(default, &default_spec, agent_configs);
    let index = select_choice(&term, label, &choices, default_choice_index(&choices))?;
    prompt_selected_agent(
        &term,
        &choices[index].value,
        &default_spec,
        agent_configs,
        models,
    )
}

fn prompt_selected_agent(
    term: &Term,
    agent: &str,
    default_spec: &agents::AgentSpec,
    agent_configs: &BTreeMap<String, AgentConfig>,
    models: &agents::ModelRoster,
) -> Result<String> {
    let spec = agents::parse_spec(agent, models)?;
    if agents::profile_for(spec.family()).is_none() || spec.model().is_some() {
        return agents::canonical_manifest_agent(&spec, agent_configs, models);
    }

    prompt_builtin_agent(term, spec.family(), default_spec, agent_configs, models)
}

fn prompt_builtin_agent(
    term: &Term,
    family: &str,
    default_spec: &agents::AgentSpec,
    agent_configs: &BTreeMap<String, AgentConfig>,
    models: &agents::ModelRoster,
) -> Result<String> {
    let default_spec = match default_spec.family() == family {
        true => Some(default_spec),
        false => None,
    };
    let model = prompt_model(term, family, default_spec, models)?;
    let effort = prompt_effort(term, family, &model, default_spec, models)?;
    let spec = agents::AgentSpec::from_parts(family, Some(&model), effort.as_deref(), models)?;
    agents::canonical_manifest_agent(&spec, agent_configs, models)
}

fn prompt_model(
    term: &Term,
    family: &str,
    default_spec: Option<&agents::AgentSpec>,
    models: &agents::ModelRoster,
) -> Result<String> {
    let choices = model_choices(family, default_spec, models);
    if choices.is_empty() {
        bail!("no {family} model options available");
    }
    let default_index = default_choice_index(&choices);
    let index = select_choice(term, &format!("{family} model"), &choices, default_index)?;
    Ok(choices[index].value.clone())
}

/// The effective roster is the whole menu: a model off it cannot be launched, so offering one
/// would be offering a spec that fails at spawn.
fn model_choices(
    family: &str,
    default_spec: Option<&agents::AgentSpec>,
    models: &agents::ModelRoster,
) -> Vec<MenuChoice<String>> {
    let default_model = default_spec.and_then(agents::AgentSpec::model);
    let mut choices = models
        .model_names(family)
        .map(|model| MenuChoice {
            label: model.to_owned(),
            value: model.to_owned(),
            is_default: default_model == Some(model),
        })
        .collect::<Vec<_>>();
    if !choices.iter().any(|choice| choice.is_default) {
        let default_model = agents::profile_for(family).map(|profile| profile.default_model);
        if let Some(choice) = choices
            .iter_mut()
            .find(|choice| Some(choice.value.as_str()) == default_model)
        {
            choice.is_default = true;
        }
    }
    choices
}

fn prompt_effort(
    term: &Term,
    family: &str,
    model: &str,
    default_spec: Option<&agents::AgentSpec>,
    models: &agents::ModelRoster,
) -> Result<Option<String>> {
    let efforts = models
        .supported_efforts(family, model)
        .with_context(|| format!("model `{model}` disappeared from the {family} roster"))?;
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
    choices.extend(efforts.iter().map(|effort| MenuChoice {
        label: effort.clone(),
        value: Some(effort.clone()),
        is_default: default_effort == Some(effort.as_str()),
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

    #[test]
    fn agent_choices_do_not_include_free_text_escape_hatch() {
        let models = agents::ModelRoster::builtin().unwrap();
        let default = agents::parse_spec("codex", &models).unwrap();
        let choices = agent_choices("codex", &default, &BTreeMap::new());
        let labels = choices
            .iter()
            .map(|choice| choice.label.as_str())
            .collect::<Vec<_>>();

        assert_eq!(labels, ["codex", "claude", "hermes"]);
        assert_eq!(default_choice_index(&choices), 0);
    }

    #[test]
    fn bare_families_default_to_their_profile_models() {
        let models = agents::ModelRoster::builtin().unwrap();
        for (family, expected) in [
            ("codex", "gpt-5.5"),
            ("claude", "opus"),
            ("hermes", "tencent/hy3"),
        ] {
            let choices = model_choices(family, None, &models);
            assert_eq!(choices[default_choice_index(&choices)].value, expected);
        }
    }

    #[test]
    fn explicit_model_selection_stores_model_with_default_effort() {
        let models = agents::ModelRoster::builtin().unwrap();
        let spec =
            agents::AgentSpec::from_parts("codex", Some("gpt-6-astra"), None, &models).unwrap();
        assert_eq!(spec.canonical(), "codex:gpt-6-astra");
        let spec =
            agents::AgentSpec::from_parts("codex", Some("gpt-6-astra"), Some("ultra"), &models)
                .unwrap();
        assert_eq!(spec.canonical(), "codex:gpt-6-astra:ultra");
    }

    #[test]
    fn the_manifests_model_is_the_default_choice() {
        let models = agents::ModelRoster::builtin().unwrap();
        let default = agents::parse_spec("codex:gpt-5.6-luna:max", &models).unwrap();
        let choices = model_choices("codex", Some(&default), &models);

        assert_eq!(
            choices[default_choice_index(&choices)].value,
            "gpt-5.6-luna"
        );
    }

    /// A manifest written before a model left the roster names one the menu no longer carries.
    /// The menu is the roster, so the stale model is not offered — it falls back to the first
    /// choice rather than smuggling an unlaunchable spec into the picker.
    #[test]
    fn a_model_off_the_roster_is_not_offered() {
        let models = agents::ModelRoster::builtin().unwrap();
        let default = agents::parse_spec("codex:gpt-5.4:high", &models).unwrap();
        let choices = model_choices("codex", Some(&default), &models);

        assert!(choices.iter().all(|choice| choice.value != "gpt-5.4"));
        assert_eq!(choices[default_choice_index(&choices)].value, "gpt-5.5");
    }
}
