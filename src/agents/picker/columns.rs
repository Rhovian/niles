use anyhow::{Context, Result};
use ratatui::crossterm::event::KeyCode;

use crate::{
    agents::{self, AgentSpec, InvocationDefaults},
    config::spec::ProjectConfig,
    util::find_on_path,
};

#[derive(Clone)]
pub(super) struct Family {
    pub name: String,
    pub installed: bool,
    pub models: Vec<Model>,
}

#[derive(Clone)]
pub(super) struct Model {
    pub name: String,
    pub efforts: Vec<String>,
}

pub(super) fn families(config: &ProjectConfig) -> Result<Vec<Family>> {
    let mut names = agents::known_agent_ids()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for name in config.agents.keys() {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    names
        .into_iter()
        .map(|name| {
            let invocation = agents::invocation(
                &name,
                agents::config_for(&config.agents, &name, &config.models)?,
                InvocationDefaults::Worker,
                &config.models,
            )?;
            let models = config
                .models
                .model_names(&name)
                .map(|model| {
                    Ok(Model {
                        name: model.to_owned(),
                        efforts: config
                            .models
                            .supported_efforts(&name, model)
                            .context("listed model has no roster entry")?
                            .to_vec(),
                    })
                })
                .collect::<Result<_>>()?;
            Ok(Family {
                name,
                installed: find_on_path(&invocation.binary).is_some(),
                models,
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Column {
    Family,
    Model,
    Effort,
}

pub(super) enum Pick {
    Pending,
    Cancelled,
    Selected(String),
}

/// Selections are tentative until the last column completes. Back navigation never edits a role.
pub(crate) struct Columns {
    pub(super) families: Vec<Family>,
    pub(super) family: usize,
    pub(super) model: usize,
    pub(super) effort: usize,
    pub(super) column: Column,
    current: AgentSpec,
}

impl Columns {
    #[expect(
        clippy::disallowed_methods,
        reason = "the picker boundary selects the first family if the current one is no longer configured"
    )]
    pub(super) fn new(families: &[Family], current: &str, config: &ProjectConfig) -> Result<Self> {
        let families = families.to_vec();
        let current = AgentSpec::parse(current, &config.models)?;
        let name = current.family();
        let family = families
            .iter()
            .position(|family| family.name == name)
            .unwrap_or(FIRST);
        let mut picker = Self {
            families,
            family,
            model: FIRST,
            effort: FIRST,
            column: Column::Family,
            current,
        };
        picker.select_model();
        Ok(picker)
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "unbound terminal keys leave the selection unchanged"
    )]
    pub(super) fn key(&mut self, key: KeyCode) -> Pick {
        match key {
            KeyCode::Up | KeyCode::Down => {
                let (index, len) = match self.column {
                    Column::Family => (&mut self.family, self.families.len()),
                    Column::Model => (&mut self.model, self.families[self.family].models.len()),
                    Column::Effort => (
                        &mut self.effort,
                        self.families[self.family].models[self.model].efforts.len() + 1,
                    ),
                };
                *index = if key == KeyCode::Up {
                    index.saturating_sub(1)
                } else {
                    (*index + 1).min(len - 1)
                };
                match self.column {
                    Column::Family => self.select_model(),
                    Column::Model => self.select_effort(),
                    Column::Effort => {}
                }
            }
            KeyCode::Left | KeyCode::Esc => {
                self.column = match self.column {
                    Column::Family => return Pick::Cancelled,
                    Column::Model => Column::Family,
                    Column::Effort => Column::Model,
                };
            }
            KeyCode::Right | KeyCode::Enter => {
                let family = &self.families[self.family];
                match self.column {
                    Column::Family if family.models.is_empty() => {
                        return Pick::Selected(family.name.clone());
                    }
                    Column::Family => self.column = Column::Model,
                    Column::Model if !family.models[self.model].efforts.is_empty() => {
                        self.column = Column::Effort
                    }
                    Column::Model | Column::Effort => {
                        let model = &family.models[self.model];
                        let mut value = format!("{}:{}", family.name, model.name);
                        if self.column == Column::Effort
                            && let Some(effort) = model.efforts.get(self.effort)
                        {
                            value.push(':');
                            value.push_str(effort);
                        }
                        return Pick::Selected(value);
                    }
                }
            }
            _ => {}
        }
        Pick::Pending
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "a missing current or profile model selects the first effective roster entry"
    )]
    fn select_model(&mut self) {
        let family = &self.families[self.family];
        let current = Some(&self.current).filter(|spec| spec.family() == family.name);
        let model = current
            .and_then(AgentSpec::model)
            .or_else(|| agents::profile_for(&family.name).map(|profile| profile.default_model));
        self.model = family
            .models
            .iter()
            .position(|entry| Some(entry.name.as_str()) == model)
            .unwrap_or(FIRST);
        self.select_effort();
    }

    fn select_effort(&mut self) {
        let family = &self.families[self.family];
        let Some(model) = family.models.get(self.model) else {
            return;
        };
        let effort = Some(&self.current)
            .filter(|spec| {
                spec.family() == family.name && spec.model() == Some(model.name.as_str())
            })
            .and_then(AgentSpec::effort);
        self.effort = match model
            .efforts
            .iter()
            .position(|entry| Some(entry.as_str()) == effort)
        {
            Some(index) => index,
            None => model.efforts.len(),
        };
    }
}

const FIRST: usize = 0;
