use crate::agents::{self, AgentSpec};
use crate::config::spec::ProjectConfig;
use crate::selection::Selection;
use crate::util::find_on_path;
use anyhow::{Context, Result};
use ratatui::crossterm::event::KeyCode;

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
    agents::profiles()
        .map(|profile| {
            let name = profile.id;
            let models = config
                .models
                .model_names(name)
                .map(|model| {
                    Ok(Model {
                        name: model.to_owned(),
                        efforts: config
                            .models
                            .supported_efforts(name, model)
                            .context("listed model has no roster entry")?
                            .to_vec(),
                    })
                })
                .collect::<Result<_>>()?;
            Ok(Family {
                name: name.to_owned(),
                installed: find_on_path(profile.binary).is_some(),
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

/// Selections are tentative until the last column completes. Back navigation never edits a binding.
pub(crate) struct Columns {
    pub(super) families: Selection<Family>,
    pub(super) models: Option<Selection<Model>>,
    pub(super) efforts: Option<Selection<String>>,
    pub(super) column: Column,
    current: AgentSpec,
}

impl Columns {
    pub(super) fn new(families: &[Family], current: &str, config: &ProjectConfig) -> Result<Self> {
        let current = AgentSpec::parse(current, &config.models)?;
        let mut families =
            Selection::new(families.to_vec()).context("picker has no agent families")?;
        families.prefer(|family| family.name == current.family());
        let mut picker = Self {
            families,
            models: None,
            efforts: None,
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
                let forward = key == KeyCode::Down;
                match self.column {
                    Column::Family => {
                        self.families.move_focus(forward);
                        self.select_model();
                    }
                    Column::Model => {
                        if let Some(models) = &mut self.models {
                            models.move_focus(forward);
                        }
                        self.select_effort();
                    }
                    Column::Effort => {
                        if let Some(efforts) = &mut self.efforts {
                            efforts.move_focus(forward);
                        }
                    }
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
                let family = self.families.current();
                match (&self.column, &self.models, &self.efforts) {
                    (Column::Family, Some(_), _) => self.column = Column::Model,
                    (Column::Model, _, Some(_)) => self.column = Column::Effort,
                    (_, models, efforts) => {
                        let mut value = family.name.clone();
                        if let Some(models) = models {
                            value.push(':');
                            value.push_str(&models.current().name);
                        }
                        if self.column == Column::Effort
                            && let Some(efforts) = efforts
                        {
                            value.push(':');
                            value.push_str(efforts.current());
                        }
                        return Pick::Selected(value);
                    }
                }
            }
            _ => return Pick::Pending,
        }
        Pick::Pending
    }

    fn select_model(&mut self) {
        let family = self.families.current();
        let model = Some(&self.current)
            .filter(|spec| spec.family() == family.name)
            .and_then(AgentSpec::model);
        self.models = Selection::new(family.models.clone());
        if let Some(models) = &mut self.models {
            models.prefer(|entry| Some(entry.name.as_str()) == model);
        }
        self.select_effort();
    }

    fn select_effort(&mut self) {
        self.efforts = self.models.as_ref().and_then(|models| {
            let model = models.current();
            let effort = Some(&self.current)
                .filter(|spec| {
                    spec.family() == self.families.current().name
                        && spec.model() == Some(model.name.as_str())
                })
                .and_then(AgentSpec::effort);
            let mut efforts = Selection::new(model.efforts.clone())?;
            efforts.prefer(|entry| Some(entry.as_str()) == effort);
            Some(efforts)
        });
    }
}
