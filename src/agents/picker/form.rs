use anyhow::Result;
use ratatui::crossterm::event::KeyCode;

use super::{
    Role,
    columns::{self, Columns, Family, Pick},
    presets::{self, Preset},
};
use crate::{config::spec::ProjectConfig, workspace_manifest::WorkspaceManifest};

pub(super) enum Screen {
    Roles,
    Presets { selected: usize },
    Editing { role: Role, columns: Columns },
    Review,
}

pub(super) enum Action {
    Continue,
    Save,
    Quit,
}

pub(super) struct Form {
    pub original: WorkspaceManifest,
    pub draft: WorkspaceManifest,
    pub screen: Screen,
    pub selected: usize,
    pub presets: Vec<Preset>,
    families: Vec<Family>,
}

impl Form {
    pub fn new(manifest: WorkspaceManifest, config: &ProjectConfig) -> Result<Self> {
        Ok(Self {
            original: manifest.clone(),
            draft: manifest,
            screen: Screen::Roles,
            selected: 0,
            presets: presets::load(config)?,
            families: columns::families(config)?,
        })
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "each screen ignores unbound terminal keys"
    )]
    pub fn key(&mut self, key: KeyCode, config: &ProjectConfig) -> Result<Action> {
        if key == KeyCode::Char('q') {
            return Ok(Action::Quit);
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
                KeyCode::Enter => return Ok(Action::Save),
                KeyCode::Esc | KeyCode::Left => self.screen = Screen::Roles,
                _ => {}
            },
            Screen::Presets { selected } => match key {
                KeyCode::Up => *selected = selected.saturating_sub(1),
                KeyCode::Down => *selected = (*selected + 1).min(self.presets.len() - 1),
                KeyCode::Esc | KeyCode::Left => self.screen = Screen::Roles,
                KeyCode::Enter => {
                    if let Ok(values) = &self.presets[*selected].values {
                        for (role, value) in Role::ALL.into_iter().zip(values) {
                            role.set(&mut self.draft, value.clone());
                        }
                        self.screen = Screen::Roles;
                    }
                }
                _ => {}
            },
            Screen::Roles => match key {
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => self.selected = (self.selected + 1).min(Role::ALL.len()),
                KeyCode::Char('p') => self.screen = Screen::Presets { selected: 0 },
                KeyCode::Char('s') => self.screen = Screen::Review,
                KeyCode::Enter if self.selected == 0 => {
                    self.screen = Screen::Presets { selected: 0 }
                }
                KeyCode::Enter => {
                    let role = Role::ALL[self.selected - 1];
                    self.screen = Screen::Editing {
                        role,
                        columns: Columns::new(
                            &self.families,
                            &role.value(&self.draft),
                            role == Role::Reviewer,
                            config,
                        )?,
                    };
                }
                _ => {}
            },
        }
        Ok(Action::Continue)
    }
}
