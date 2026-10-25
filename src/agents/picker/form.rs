use anyhow::{Context, Result};
use ratatui::crossterm::event::KeyCode;

use super::{
    Choice,
    columns::{self, Columns, Family, Pick},
};
use crate::skills::SkillName;
use crate::{config::spec::ProjectConfig, workspace_manifest::WorkspaceManifest};

pub(super) enum Screen {
    Skills,
    Editing { skill: SkillName, columns: Columns },
    Review,
}

pub(super) struct Form {
    pub original: WorkspaceManifest,
    pub draft: WorkspaceManifest,
    pub screen: Screen,
    pub selected: usize,
    pub save_reason: Option<String>,
    families: Vec<Family>,
}

impl Form {
    pub fn new(manifest: WorkspaceManifest, config: &ProjectConfig) -> Result<Self> {
        Ok(Self {
            original: manifest.clone(),
            draft: manifest,
            screen: Screen::Skills,
            selected: 0,
            save_reason: None,
            families: columns::families(config)?,
        })
    }

    pub fn can_save(&self) -> bool {
        self.save_reason.is_some() || self.draft != self.original
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "each screen ignores unbound terminal keys"
    )]
    /// The operator's choice, or `None` while the form stays open.
    pub fn key(&mut self, key: KeyCode, config: &ProjectConfig) -> Result<Option<Choice>> {
        match key {
            KeyCode::Char('c') if !self.can_save() => {
                return Ok(Some(Choice::Keep));
            }
            KeyCode::Char('q') => return Ok(Some(Choice::Quit)),
            _ => {}
        }
        match &mut self.screen {
            Screen::Editing { skill, columns } => match columns.key(key) {
                Pick::Pending => {}
                Pick::Cancelled => self.screen = Screen::Skills,
                Pick::Selected(value) => {
                    self.draft.set_primary(skill, value)?;
                    self.screen = Screen::Skills;
                }
            },
            Screen::Review => match key {
                KeyCode::Enter => return Ok(Some(Choice::Save(Box::new(self.draft.clone())))),
                KeyCode::Esc | KeyCode::Left => self.screen = Screen::Skills,
                _ => {}
            },
            Screen::Skills => match key {
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.draft.bindings.len() - 1)
                }
                KeyCode::Char('s') if self.can_save() => self.screen = Screen::Review,
                KeyCode::Enter => {
                    let (skill, binding) = self
                        .draft
                        .ordered_bindings()
                        .nth(self.selected)
                        .context("selected skill is no longer bound")?;
                    self.screen = Screen::Editing {
                        skill: skill.clone(),
                        columns: Columns::new(&self.families, &binding.standard.primary, config)?,
                    };
                }
                _ => {}
            },
        }
        Ok(None)
    }
}
