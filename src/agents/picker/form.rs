use anyhow::Result;
use ratatui::crossterm::event::KeyCode;

use super::columns::{self, Columns, Family, Pick};
use crate::{
    agents::roster::Tier,
    config::spec::ProjectConfig,
    workspace_manifest::{self, TierBinding, WorkspaceManifest},
};

const FIRST_ENTRY_AGENT: usize = 0;

pub(super) const TIERS: [Tier; 4] = [
    Tier::Standard,
    Tier::Frontier,
    Tier::Economy,
    Tier::Mechanical,
];

#[derive(Clone, Copy)]
pub(super) enum Row {
    Agent { tier: Tier, index: usize },
    Entry,
}

impl Row {
    pub fn value(self, manifest: &WorkspaceManifest) -> Option<&str> {
        match self {
            Self::Entry => Some(&manifest.entry_agent),
            Self::Agent { tier, index } => manifest.tiers.get(tier).and_then(|binding| {
                std::iter::once(binding.primary.as_str())
                    .chain(binding.secondaries.iter().map(String::as_str))
                    .nth(index)
            }),
        }
    }
}

pub(super) enum Screen {
    Tiers,
    Editing {
        tier: Tier,
        index: usize,
        columns: Columns,
    },
    Entry {
        selected: usize,
    },
    Review,
}

pub(super) enum Decision {
    Save,
    Keep,
    Quit,
}

pub(super) struct Form {
    pub setup: bool,
    pub original: WorkspaceManifest,
    pub draft: WorkspaceManifest,
    pub screen: Screen,
    pub selected: usize,
    pub error: String,
    families: Vec<Family>,
}

impl Form {
    pub fn new(manifest: WorkspaceManifest, config: &ProjectConfig) -> Result<Self> {
        Ok(Self {
            setup: false,
            original: manifest.clone(),
            draft: manifest,
            screen: Screen::Tiers,
            selected: 0,
            error: String::new(),
            families: columns::families(config)?,
        })
    }

    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for tier in TIERS {
            let count = match self.draft.tiers.get(tier) {
                Some(binding) => binding.secondaries.len() + 1,
                None => 1,
            };
            rows.extend((0..count).map(|index| Row::Agent { tier, index }));
        }
        rows.push(Row::Entry);
        rows
    }

    pub fn entry_choices(&self) -> Vec<&str> {
        let mut agents = Vec::new();
        for agent in self.draft.tiers.agents() {
            if !agents.contains(&agent) {
                agents.push(agent);
            }
        }
        agents
    }

    pub fn can_save(&self) -> bool {
        self.draft != self.original
    }

    fn accept(&mut self, config: &ProjectConfig) -> bool {
        match workspace_manifest::check(&self.draft, &config.models) {
            Ok(()) => {
                self.error.clear();
                true
            }
            Err(error) => {
                self.error = format!("{error:#}");
                false
            }
        }
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "each screen ignores unbound terminal keys"
    )]
    pub fn key(&mut self, key: KeyCode, config: &ProjectConfig) -> Result<Option<Decision>> {
        if key == KeyCode::Char('q') {
            return Ok(Some(Decision::Quit));
        }
        match &mut self.screen {
            Screen::Editing {
                tier,
                index,
                columns,
            } => match columns.key(key) {
                Pick::Pending => {}
                Pick::Cancelled => self.screen = Screen::Tiers,
                Pick::Selected(value) => {
                    let tier = *tier;
                    let index = *index;
                    let mut binding = match self.draft.tiers.get(tier) {
                        Some(binding) => binding.clone(),
                        None => TierBinding {
                            primary: value.clone(),
                            secondaries: Vec::new(),
                        },
                    };
                    debug_assert!(index <= binding.secondaries.len() + 1);
                    let selected = crate::agents::AgentSpec::parse(&value, &config.models)?;
                    let mut duplicate = false;
                    for (i, agent) in std::iter::once(&binding.primary)
                        .chain(&binding.secondaries)
                        .enumerate()
                    {
                        if i != index
                            && crate::agents::AgentSpec::parse(agent, &config.models)? == selected
                        {
                            duplicate = true;
                            break;
                        }
                    }
                    if duplicate {
                        self.error = format!("Duplicate agent: {value}");
                    } else {
                        if index == 0 {
                            binding.primary = value;
                        } else if index <= binding.secondaries.len() {
                            binding.secondaries[index - 1] = value;
                        } else {
                            binding.secondaries.push(value);
                        }
                        self.draft.tiers.set(tier, binding);
                        self.error.clear();
                    }
                    self.screen = Screen::Tiers;
                }
            },
            Screen::Entry { selected } => {
                let mut cursor = *selected;
                let choices = self.entry_choices();
                match key {
                    KeyCode::Up => cursor = cursor.saturating_sub(1),
                    KeyCode::Down => cursor = (cursor + 1).min(choices.len() - 1),
                    KeyCode::Enter => {
                        self.draft.entry_agent = choices[cursor].to_owned();
                        self.screen = Screen::Tiers;
                        self.error.clear();
                        return Ok(None);
                    }
                    KeyCode::Esc | KeyCode::Left => {
                        self.screen = Screen::Tiers;
                        return Ok(None);
                    }
                    _ => {}
                }
                self.screen = Screen::Entry { selected: cursor };
            }
            Screen::Review => match key {
                KeyCode::Enter if self.accept(config) => {
                    return Ok(Some(Decision::Save));
                }
                KeyCode::Esc | KeyCode::Left => self.screen = Screen::Tiers,
                _ => {}
            },
            Screen::Tiers => match key {
                KeyCode::Char('c') if !self.can_save() => return Ok(Some(Decision::Keep)),
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => self.selected = (self.selected + 1).min(self.rows().len() - 1),
                KeyCode::Char('s') if self.can_save() && self.accept(config) => {
                    self.screen = Screen::Review
                }
                KeyCode::Enter if self.setup => {
                    if self.accept(config) {
                        return Ok(Some(Decision::Save));
                    }
                }
                KeyCode::Right | KeyCode::Enter | KeyCode::Char('a') => {
                    match self.rows()[self.selected] {
                        Row::Entry => {
                            let selected = match self
                                .entry_choices()
                                .iter()
                                .position(|a| *a == self.draft.entry_agent)
                            {
                                Some(index) => index,
                                None => FIRST_ENTRY_AGENT,
                            };
                            self.screen = Screen::Entry { selected };
                        }
                        Row::Agent { tier, index } => {
                            let (index, current): (usize, &str) = match self.draft.tiers.get(tier) {
                                Some(binding) if key == KeyCode::Char('a') => {
                                    (binding.secondaries.len() + 1, binding.primary.as_str())
                                }
                                Some(binding) => (
                                    index,
                                    if index == 0 {
                                        &binding.primary
                                    } else {
                                        &binding.secondaries[index - 1]
                                    },
                                ),
                                None => (0, self.draft.entry_agent.as_str()),
                            };
                            self.screen = Screen::Editing {
                                tier,
                                index,
                                columns: Columns::new(&self.families, current, config)?,
                            };
                        }
                    }
                }
                KeyCode::Char('d' | '[' | ']') => {
                    if let Row::Agent { tier, index } = self.rows()[self.selected]
                        && index > 0
                    {
                        let mut binding = self.draft.tiers.tier(tier)?.clone();
                        let i = index - 1;
                        match key {
                            KeyCode::Char('d') => {
                                binding.secondaries.remove(i);
                                self.selected -= 1;
                            }
                            KeyCode::Char('[') if i > 0 => {
                                binding.secondaries.swap(i, i - 1);
                                self.selected -= 1;
                            }
                            KeyCode::Char(']') if i + 1 < binding.secondaries.len() => {
                                binding.secondaries.swap(i, i + 1);
                                self.selected += 1;
                            }
                            _ => {}
                        }
                        self.draft.tiers.set(tier, binding);
                    }
                }
                _ => {}
            },
        }
        Ok(None)
    }
}
