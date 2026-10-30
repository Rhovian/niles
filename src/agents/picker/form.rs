use anyhow::{Context, Result};
use ratatui::crossterm::event::KeyCode;

use super::{
    Choice,
    columns::{self, Columns, Family, Pick},
};
use crate::agents::roster::Tier;
use crate::{config::spec::ProjectConfig, workspace_manifest::WorkspaceManifest};

pub(super) enum Screen {
    Tiers,
    Editing { tier: Tier, columns: Columns },
    Review,
}

pub(super) struct Form {
    pub original: WorkspaceManifest,
    pub draft: WorkspaceManifest,
    pub screen: Screen,
    pub selected: usize,
    families: Vec<Family>,
}

impl Form {
    pub fn new(manifest: WorkspaceManifest, config: &ProjectConfig) -> Result<Self> {
        Ok(Self {
            original: manifest.clone(),
            draft: manifest,
            screen: Screen::Tiers,
            selected: 0,
            families: columns::families(config)?,
        })
    }

    pub fn can_save(&self) -> bool {
        self.draft != self.original
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
            Screen::Editing { tier, columns } => match columns.key(key) {
                Pick::Pending => {}
                Pick::Cancelled => self.screen = Screen::Tiers,
                Pick::Selected(value) => {
                    self.draft.tiers.set_primary(*tier, value)?;
                    self.screen = Screen::Tiers;
                }
            },
            Screen::Review => match key {
                KeyCode::Enter => return Ok(Some(Choice::Save(Box::new(self.draft.clone())))),
                KeyCode::Esc | KeyCode::Left => self.screen = Screen::Tiers,
                _ => {}
            },
            Screen::Tiers => match key {
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.draft.tiers.iter().count() - 1)
                }
                KeyCode::Char('s') if self.can_save() => self.screen = Screen::Review,
                KeyCode::Enter => {
                    let (tier, binding) = self
                        .draft
                        .tiers
                        .iter()
                        .nth(self.selected)
                        .context("selected tier is no longer listed")?;
                    self.screen = Screen::Editing {
                        tier,
                        columns: Columns::new(&self.families, &binding.primary, config)?,
                    };
                }
                _ => {}
            },
        }
        Ok(None)
    }
}
