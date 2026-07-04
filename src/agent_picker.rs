use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, Write},
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

mod catalog;

use catalog::{
    ModelCatalog, ModelGroup, effort_options, insert_model_group, model_catalog,
    write_catalog_source,
};

use crate::config::{agents, spec::AgentConfig};

pub(crate) fn prompt_agent_value<R: BufRead, W: Write>(
    root: &Utf8Path,
    input: &mut R,
    output: &mut W,
    label: &str,
    default: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<String> {
    loop {
        let choices = agent_choices(default, agent_configs);
        write_menu(output, label, &choices)?;
        let line = read_prompt_line(input, output, &format!("Select {label} [{default}]: "))?;

        if line.is_empty() {
            return canonical_manifest_agent(default, agent_configs);
        }

        match line.parse::<usize>() {
            Ok(number) if (1..=choices.len()).contains(&number) => {
                let choice = choices[number - 1].value.clone();
                return match choice {
                    AgentChoice::Preset(agent) => {
                        prompt_selected_agent(root, input, output, &agent, default, agent_configs)
                    }
                    AgentChoice::Other => {
                        prompt_custom_agent_spec(input, output, default, agent_configs)
                    }
                };
            }
            Ok(_) => writeln!(output, "Please choose 1-{}.", choices.len())?,
            Err(_) => match canonical_manifest_agent(&line, agent_configs) {
                Ok(value) => return Ok(value),
                Err(err) => writeln!(output, "Invalid agent spec: {err}")?,
            },
        }
    }
}

fn prompt_selected_agent<R: BufRead, W: Write>(
    root: &Utf8Path,
    input: &mut R,
    output: &mut W,
    agent: &str,
    default: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<String> {
    let spec = agents::parse_spec(agent)?;
    if agents::profile_for(spec.family()).is_none() || spec.model().is_some() {
        return canonical_manifest_agent(agent, agent_configs);
    }

    prompt_builtin_agent(root, input, output, spec.family(), default, agent_configs)
}

fn prompt_builtin_agent<R: BufRead, W: Write>(
    root: &Utf8Path,
    input: &mut R,
    output: &mut W,
    family: &str,
    default: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<String> {
    let default_spec = agents::parse_spec(default).ok();
    let default_spec = default_spec.as_ref().filter(|spec| spec.family() == family);
    let catalog = model_catalog(root, family, agent_configs)?;
    write_catalog_source(output, family, &catalog)?;

    let selection = prompt_model(input, output, family, default_spec, &catalog, agent_configs)?;
    let model = match selection {
        ModelSelection::Default => return Ok(family.to_owned()),
        ModelSelection::Model(model) => model,
        ModelSelection::Custom(value) => return Ok(value),
    };

    let effort = prompt_effort(input, output, family, &model, default_spec, &catalog)?;
    let value = agent_value(family, Some(&model), effort.as_deref());
    canonical_manifest_agent(&value, agent_configs)
}

fn prompt_model<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    family: &str,
    default_spec: Option<&agents::AgentSpec>,
    catalog: &ModelCatalog,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<ModelSelection> {
    let mut groups = catalog.groups.clone();
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

    let default_index = choices
        .iter()
        .position(|choice| choice.is_default)
        .unwrap_or(0);
    let index = prompt_numbered_choice(
        input,
        output,
        &format!("{family} model"),
        &format!("Select {family} model [{}]: ", default_index + 1),
        &choices,
        default_index,
    )?;

    match choices[index].value.clone() {
        ModelChoice::Default => Ok(ModelSelection::Default),
        ModelChoice::Group(group) => {
            choose_model_version(input, output, family, &group, default_spec)
                .map(ModelSelection::Model)
        }
        ModelChoice::Other => prompt_custom_agent_spec(
            input,
            output,
            &agent_value(family, None, None),
            agent_configs,
        )
        .map(ModelSelection::Custom),
    }
}

fn choose_model_version<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
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
    let default_index = choices
        .iter()
        .position(|choice| choice.is_default)
        .unwrap_or(0);
    let index = prompt_numbered_choice(
        input,
        output,
        &format!("{family} {} version", group.label),
        &format!(
            "Select {family} {} version [{}]: ",
            group.label,
            default_index + 1
        ),
        &choices,
        default_index,
    )?;
    Ok(choices[index].value.clone())
}

fn prompt_effort<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
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

    let default_index = choices
        .iter()
        .position(|choice| choice.is_default)
        .unwrap_or(0);
    let index = prompt_numbered_choice(
        input,
        output,
        &format!("{family} effort"),
        &format!("Select {family} effort [{}]: ", default_index + 1),
        &choices,
        default_index,
    )?;
    Ok(choices[index].value.clone())
}

fn prompt_numbered_choice<R: BufRead, W: Write, T>(
    input: &mut R,
    output: &mut W,
    title: &str,
    prompt: &str,
    choices: &[MenuChoice<T>],
    default_index: usize,
) -> Result<usize> {
    write_menu(output, title, choices)?;
    loop {
        let line = read_prompt_line(input, output, prompt)?;
        if line.is_empty() {
            return Ok(default_index);
        }

        match line.parse::<usize>() {
            Ok(number) if (1..=choices.len()).contains(&number) => return Ok(number - 1),
            Ok(_) | Err(_) => writeln!(output, "Please choose 1-{}.", choices.len())?,
        }
    }
}

fn prompt_custom_agent_spec<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    default: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<String> {
    loop {
        let value = prompt_value(input, output, "Custom agent spec", default)?;
        match canonical_manifest_agent(&value, agent_configs) {
            Ok(value) => return Ok(value),
            Err(err) => writeln!(output, "Invalid agent spec: {err}")?,
        }
    }
}

fn prompt_value<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    label: &str,
    default: &str,
) -> Result<String> {
    loop {
        let value = read_prompt_line(input, output, &format!("{label} [{default}]: "))?;
        let value = if value.is_empty() { default } else { &value };
        if !value.trim().is_empty() {
            return Ok(value.to_owned());
        }

        writeln!(output, "{label} cannot be empty")?;
    }
}

fn read_prompt_line<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    prompt: &str,
) -> Result<String> {
    write!(output, "{prompt}")?;
    output.flush()?;

    let mut line = String::new();
    let bytes = input
        .read_line(&mut line)
        .with_context(|| format!("failed to read {prompt}"))?;
    if bytes == 0 {
        bail!("stdin closed before workspace manifest was configured");
    }

    Ok(line.trim().to_owned())
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

fn write_menu<W: Write, T>(output: &mut W, title: &str, choices: &[MenuChoice<T>]) -> Result<()> {
    writeln!(output, "{title}:")?;
    for (index, choice) in choices.iter().enumerate() {
        let marker = if choice.is_default { " (default)" } else { "" };
        writeln!(output, "  {}) {}{marker}", index + 1, choice.label)?;
    }
    Ok(())
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
mod tests;
