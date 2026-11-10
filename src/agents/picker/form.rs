use super::columns::{self, Columns, Family, Pick};
use crate::agents::AgentSpec;
use crate::agents::roster::Tier;
use crate::config::spec::ProjectConfig;
use crate::selection::Selection;
use crate::workspace_manifest::{self, TierBinding, WorkspaceManifest};
use anyhow::{Context, Result};
use ratatui::crossterm::event::KeyCode;
use std::iter;

pub(super) const TIERS: [Tier; 4] = [
    Tier::Standard,
    Tier::Frontier,
    Tier::Economy,
    Tier::Mechanical,
];

fn agents(manifest: &WorkspaceManifest, tier: Tier) -> impl Iterator<Item = Option<&str>> {
    let binding = manifest.tiers.get(tier);
    iter::once(binding.map(|binding| binding.primary.as_str())).chain(
        binding
            .into_iter()
            .flat_map(|binding| binding.secondaries.iter().map(|agent| Some(agent.as_str()))),
    )
}

pub(super) enum EditRow {
    Primary { tier: Tier, value: Option<String> },
    Secondary { tier: Tier, value: String },
    Entry,
}

impl EditRow {
    fn tier(&self) -> Option<Tier> {
        match self {
            Self::Primary { tier, .. } | Self::Secondary { tier, .. } => Some(*tier),
            Self::Entry => None,
        }
    }

    fn value(&self) -> Option<&str> {
        match self {
            Self::Primary { value, .. } => value.as_deref(),
            Self::Secondary { value, .. } => Some(value),
            Self::Entry => None,
        }
    }
}

pub(super) enum Screen {
    Tiers,
    Editing {
        tier: Tier,
        append: bool,
        columns: Box<Columns>,
    },
    Entry {
        choices: Selection<String>,
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
    pub selection: Selection<EditRow>,
    pub error: String,
    families: Vec<Family>,
}

impl Form {
    pub fn new(manifest: WorkspaceManifest, config: &ProjectConfig) -> Result<Self> {
        let rows = TIERS
            .into_iter()
            .flat_map(|tier| {
                let binding = manifest.tiers.get(tier);
                iter::once(EditRow::Primary {
                    tier,
                    value: binding.map(|b| b.primary.clone()),
                })
                .chain(binding.into_iter().flat_map(move |b| {
                    b.secondaries
                        .iter()
                        .cloned()
                        .map(move |value| EditRow::Secondary { tier, value })
                }))
            })
            .chain(iter::once(EditRow::Entry));
        let selection = Selection::new(rows).context("picker has no tier rows")?;
        Ok(Self {
            setup: false,
            original: manifest.clone(),
            draft: manifest,
            screen: Screen::Tiers,
            selection,
            error: String::new(),
            families: columns::families(config)?,
        })
    }

    pub fn rows(&self) -> impl Iterator<Item = (String, Option<&str>, Option<&str>)> {
        TIERS
            .into_iter()
            .flat_map(|tier| {
                agents(&self.draft, tier)
                    .enumerate()
                    .zip(agents(&self.original, tier).chain(iter::repeat(None)))
                    .map(move |((index, value), original)| {
                        let label = if index == 0 {
                            tier.as_str().to_owned()
                        } else {
                            format!("  secondary {index}")
                        };
                        (label, value, original)
                    })
            })
            .chain(iter::once((
                "entry_agent".to_owned(),
                Some(self.draft.entry_agent.as_str()),
                Some(self.original.entry_agent.as_str()),
            )))
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

    pub fn select_agent(&mut self, tier: Tier, index: usize) -> Result<()> {
        let position = self
            .selection
            .iter()
            .enumerate()
            .filter(|(_, row)| row.tier() == Some(tier))
            .nth(index)
            .map(|(position, _)| position)
            .context("tier row is missing")?;
        self.selection.focus(position)?;
        Ok(())
    }

    fn update_tier(&mut self, tier: Tier) -> Result<()> {
        let mut values = self
            .selection
            .iter()
            .filter(|row| row.tier() == Some(tier))
            .filter_map(EditRow::value);
        let primary = values.next().context("edited tier has no primary agent")?;
        self.draft.tiers.set(
            tier,
            TierBinding {
                primary: primary.to_owned(),
                secondaries: values.map(str::to_owned).collect(),
            },
        );
        Ok(())
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
                append,
                columns,
            } => match columns.key(key) {
                Pick::Pending => {}
                Pick::Cancelled => self.screen = Screen::Tiers,
                Pick::Selected(value) => {
                    let tier = *tier;
                    let append = *append;
                    let selected = AgentSpec::parse(&value, &config.models)?;
                    let mut duplicate = false;
                    for (position, row) in self.selection.iter().enumerate() {
                        if row.tier() == Some(tier)
                            && (append || position != self.selection.position())
                            && let Some(agent) = row.value()
                            && AgentSpec::parse(agent, &config.models)? == selected
                        {
                            duplicate = true;
                            break;
                        }
                    }
                    if duplicate {
                        self.error = format!("Duplicate agent: {value}");
                    } else {
                        if append {
                            let position = self.selection.position();
                            let end = self
                                .selection
                                .iter()
                                .enumerate()
                                .filter(|(_, row)| row.tier() == Some(tier))
                                .map(|(position, _)| position)
                                .last()
                                .context("tier has no rows")?;
                            self.selection.focus(end)?;
                            self.selection
                                .insert_after(EditRow::Secondary { tier, value });
                            self.selection.focus(position)?;
                        } else {
                            match self.selection.current_mut() {
                                EditRow::Primary { value: target, .. } => *target = Some(value),
                                EditRow::Secondary { value: target, .. } => *target = value,
                                EditRow::Entry => anyhow::bail!("agent editor selected entry row"),
                            }
                        }
                        self.update_tier(tier)?;
                        self.error.clear();
                    }
                    self.screen = Screen::Tiers;
                }
            },
            Screen::Entry { choices } => match key {
                KeyCode::Up | KeyCode::Down => choices.move_focus(key == KeyCode::Down),
                KeyCode::Enter => {
                    self.draft.entry_agent = choices.current().clone();
                    self.screen = Screen::Tiers;
                    self.error.clear();
                }
                KeyCode::Esc | KeyCode::Left => self.screen = Screen::Tiers,
                _ => return Ok(None),
            },
            Screen::Review => match key {
                KeyCode::Enter if self.accept(config) => return Ok(Some(Decision::Save)),
                KeyCode::Esc | KeyCode::Left => self.screen = Screen::Tiers,
                _ => return Ok(None),
            },
            Screen::Tiers => match key {
                KeyCode::Char('c') if !self.can_save() => return Ok(Some(Decision::Keep)),
                KeyCode::Up | KeyCode::Down => self.selection.move_focus(key == KeyCode::Down),
                KeyCode::Char('s') if self.can_save() && self.accept(config) => {
                    self.screen = Screen::Review
                }
                KeyCode::Enter if self.setup => {
                    if self.accept(config) {
                        return Ok(Some(Decision::Save));
                    }
                }
                KeyCode::Right | KeyCode::Enter | KeyCode::Char('a') => {
                    if let Some(tier) = self.selection.current().tier() {
                        let append =
                            key == KeyCode::Char('a') && self.draft.tiers.get(tier).is_some();
                        let current = if append {
                            &self.draft.tiers.tier(tier)?.primary
                        } else {
                            match self.selection.current().value() {
                                Some(value) => value,
                                None => &self.draft.entry_agent,
                            }
                        };
                        self.screen = Screen::Editing {
                            tier,
                            append,
                            columns: Box::new(Columns::new(&self.families, current, config)?),
                        };
                    } else {
                        let mut choices =
                            Selection::new(self.entry_choices().into_iter().map(str::to_owned))
                                .context("no tier agents available for entry selection")?;
                        choices.prefer(|agent| *agent == self.draft.entry_agent);
                        self.screen = Screen::Entry { choices };
                    }
                }
                KeyCode::Char('d' | '[' | ']') => {
                    if let EditRow::Secondary { tier, .. } = self.selection.current() {
                        let tier = *tier;
                        if key == KeyCode::Char('d') {
                            self.selection.remove()?;
                        } else {
                            self.selection.reorder(key == KeyCode::Char(']'), |row| {
                                matches!(row, EditRow::Secondary { tier: candidate, .. } if *candidate == tier)
                            });
                        }
                        self.update_tier(tier)?;
                    }
                }
                _ => return Ok(None),
            },
        }
        Ok(None)
    }
}
