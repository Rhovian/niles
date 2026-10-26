use std::collections::BTreeMap;

use crate::skills::SkillName;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::agents::{AgentSpec, ModelRoster, roster::Tier};

/// Skill bindings and workspace-wide dispatch settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceManifest {
    #[serde(skip)]
    pub(crate) source: Option<super::setup::SetupFile>,
    // Flatten accepts skill keys; nested Binding and TierBinding remain strict.
    #[serde(flatten)]
    pub bindings: BTreeMap<SkillName, Binding>,
    #[serde(default)]
    pub entry: SkillName,
    /// `checkin:` — the delay `spawn` and `send` arm when `--checkin` is not given: a duration such
    /// as `1s`, `90s`, `5m` or `1h`, or `off` for none. Absent is the built-in five-minute
    /// default.
    ///
    /// A value the delay speller rejects fails the dispatch, naming this file, rather than
    /// quietly falling back to the default the workspace just tried to change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkin: Option<String>,
    /// `recheck:` — what a check-in that has fired arms at next: the literal `backoff` (each fire
    /// doubles the delay, up to an hour) or a fixed delay like `10m` to re-arm flat. Absent is
    /// `backoff`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recheck: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<super::WorktreeConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<SkillName>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub always: Vec<SkillName>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_from: Vec<SkillName>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_stage: Vec<SkillName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gates: Option<Gates>,
    pub standard: TierBinding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontier: Option<TierBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub economy: Option<TierBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mechanical: Option<TierBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gates {
    #[serde(default)]
    pub rounds: Rounds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Rounds(u8);

impl Rounds {
    const MIN: u8 = 1;
    const MAX: u8 = 10;
}

impl Default for Rounds {
    fn default() -> Self {
        Self(Self::MIN)
    }
}

impl TryFrom<u8> for Rounds {
    type Error = anyhow::Error;
    fn try_from(value: u8) -> Result<Self> {
        anyhow::ensure!(
            (Self::MIN..=Self::MAX).contains(&value),
            "gates.rounds must be {}..={}",
            Self::MIN,
            Self::MAX
        );
        Ok(Self(value))
    }
}

impl From<Rounds> for u8 {
    fn from(rounds: Rounds) -> Self {
        rounds.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TierBinding {
    pub primary: String,
    pub secondaries: Vec<String>,
}

impl WorkspaceManifest {
    pub fn binding(&self, skill: &SkillName) -> Result<&Binding> {
        self.bindings.get(skill).ok_or_else(|| {
            anyhow::anyhow!(
                "skill {skill} is not bound; bound skills: {}",
                self.bindings
                    .keys()
                    .map(SkillName::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
    }

    pub(crate) fn set_primary(&mut self, skill: &SkillName, primary: String) -> Result<()> {
        self.bindings
            .get_mut(skill)
            .with_context(|| format!("skill {skill} is no longer bound"))?
            .standard
            .primary = primary;
        Ok(())
    }

    pub(crate) fn ordered_bindings(&self) -> impl Iterator<Item = (&SkillName, &Binding)> {
        self.bindings.get_key_value(&self.entry).into_iter().chain(
            self.bindings
                .iter()
                .filter(|(skill, _)| **skill != self.entry),
        )
    }
}

impl Binding {
    pub fn tier(&self, tier: Tier) -> Result<&TierBinding> {
        match tier {
            Tier::Standard => Some(&self.standard),
            Tier::Frontier => self.frontier.as_ref(),
            Tier::Economy => self.economy.as_ref(),
            Tier::Mechanical => self.mechanical.as_ref(),
        }
        .ok_or_else(|| anyhow::anyhow!("skill does not list tier {}", tier.as_str()))
    }

    pub fn allows(&self, requested: &AgentSpec, models: &ModelRoster) -> Result<bool> {
        for agent in self.agents() {
            if exact_spec(agent, models)? == *requested {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn tiers(&self) -> impl Iterator<Item = (Tier, &TierBinding)> {
        [
            Some((Tier::Standard, &self.standard)),
            self.frontier
                .as_ref()
                .map(|binding| (Tier::Frontier, binding)),
            self.economy
                .as_ref()
                .map(|binding| (Tier::Economy, binding)),
            self.mechanical
                .as_ref()
                .map(|binding| (Tier::Mechanical, binding)),
        ]
        .into_iter()
        .flatten()
    }

    pub fn agents(&self) -> impl Iterator<Item = &str> {
        self.tiers().flat_map(|(_, tier)| {
            std::iter::once(tier.primary.as_str())
                .chain(tier.secondaries.iter().map(String::as_str))
        })
    }
}

pub(super) fn exact_spec(agent: &str, models: &ModelRoster) -> Result<AgentSpec> {
    let spec = AgentSpec::parse(agent, models)?;
    let Some(model) = spec.model() else {
        bail!("manifest agent `{agent}` must name an exact family:model:effort");
    };
    if spec.effort().is_none()
        && models
            .supported_efforts(spec.family(), model)
            .is_none_or(|efforts| !efforts.is_empty())
    {
        bail!("manifest agent `{agent}` must name an exact effort");
    }
    crate::agents::validate_model(&spec, models)?;
    Ok(spec)
}

impl Default for WorkspaceManifest {
    #[expect(
        clippy::expect_used,
        reason = "the shipped manifest is compiled in and tested"
    )]
    fn default() -> Self {
        crate::store::parse_yaml(include_str!("../../examples/claude-lead.yaml"))
            .expect("shipped manifest must parse")
    }
}

#[cfg(test)]
impl From<String> for Binding {
    fn from(primary: String) -> Self {
        Self {
            skills: Vec::new(),
            always: Vec::new(),
            on_from: Vec::new(),
            on_stage: Vec::new(),
            gates: None,
            standard: TierBinding {
                primary,
                secondaries: Vec::new(),
            },
            frontier: None,
            economy: None,
            mechanical: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_are_bounded_at_yaml_and_json_parse_boundaries() {
        for value in [0, 11, 256, -1] {
            let body = format!("rounds: {value}");
            assert!(crate::store::parse_yaml::<Gates>(&body).is_err(), "{body}");
            assert!(serde_json::from_value::<Gates>(serde_json::json!({"rounds": value})).is_err());
        }
        for value in [1, 10] {
            let gates: Gates = crate::store::parse_yaml(&format!("rounds: {value}")).unwrap();
            assert_eq!(u8::from(gates.rounds), value);
        }
        let gates: Gates = crate::store::parse_yaml("{}").unwrap();
        assert_eq!(u8::from(gates.rounds), 1);
        assert!(crate::store::parse_yaml::<Gates>("typo: 1").is_err());
    }
}
