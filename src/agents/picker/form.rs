use anyhow::Result;
use ratatui::crossterm::event::KeyCode;

use super::{
    Choice, Role,
    columns::{self, Columns, Family, Pick},
};
use crate::{config::spec::ProjectConfig, workspace_manifest::WorkspaceManifest};

pub(super) enum Screen {
    Roles,
    Editing { role: Role, columns: Columns },
    Review,
}

pub(super) struct Form {
    pub original: WorkspaceManifest,
    pub draft: WorkspaceManifest,
    pub screen: Screen,
    pub selected: usize,
    pub replacement_reason: Option<String>,
    families: Vec<Family>,
}

impl Form {
    pub fn new(manifest: WorkspaceManifest, config: &ProjectConfig) -> Result<Self> {
        Ok(Self {
            original: manifest.clone(),
            draft: manifest,
            screen: Screen::Roles,
            selected: 0,
            replacement_reason: None,
            families: columns::families(config)?,
        })
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "each screen ignores unbound terminal keys"
    )]
    /// The operator's choice, or `None` while the form stays open.
    pub fn key(&mut self, key: KeyCode, config: &ProjectConfig) -> Result<Option<Choice>> {
        match key {
            KeyCode::Char('c') if self.replacement_reason.is_none() => {
                return Ok(Some(Choice::Keep));
            }
            KeyCode::Char('q') => return Ok(Some(Choice::Quit)),
            _ => {}
        }
        match &mut self.screen {
            Screen::Editing { role, columns } => match columns.key(key) {
                Pick::Pending => {}
                Pick::Cancelled => self.screen = Screen::Roles,
                Pick::Selected(value) => {
                    role.set(&mut self.draft, value);
                    self.screen = Screen::Roles;
                }
            },
            Screen::Review => match key {
                KeyCode::Enter => return Ok(Some(Choice::Save(self.draft.clone()))),
                KeyCode::Esc | KeyCode::Left => self.screen = Screen::Roles,
                _ => {}
            },
            Screen::Roles => match key {
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => self.selected = (self.selected + 1).min(Role::ALL.len() - 1),
                KeyCode::Char('s') => self.screen = Screen::Review,
                KeyCode::Enter => {
                    let role = Role::ALL[self.selected];
                    self.screen = Screen::Editing {
                        role,
                        columns: Columns::new(&self.families, &role.value(&self.draft), config)?,
                    };
                }
                _ => {}
            },
        }
        Ok(None)
    }
}
