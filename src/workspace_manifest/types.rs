use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::agents::{AgentSpec, ModelRoster, roster::Tier};

/// Which agent plays each role, plus check-in settings.
///
/// Every role that has its own brief has its own binding, `security` included: it is
/// commissioned rarely, but when it is, the tier it runs at is a workspace decision rather
/// than something the lead should have to remember per spawn.
///
/// The check-in keys are workspace-wide rather than per role: they say how often the lead is
/// nudged about a worker that has gone quiet, which is a property of the workspace's pace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceManifest {
    pub lead: RoleBinding,
    pub worker: RoleBinding,
    pub reviewer: RoleBinding,
    pub security: RoleBinding,
    pub design: RoleBinding,
    pub research: RoleBinding,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleBinding {
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
pub struct TierBinding {
    pub primary: String,
    pub secondaries: Vec<String>,
}

impl RoleBinding {
    pub fn tier(&self, tier: Tier) -> Result<&TierBinding> {
        match tier {
            Tier::Standard => Some(&self.standard),
            Tier::Frontier => self.frontier.as_ref(),
            Tier::Economy => self.economy.as_ref(),
            Tier::Mechanical => self.mechanical.as_ref(),
        }
        .ok_or_else(|| anyhow::anyhow!("role does not list tier {}", tier.as_str()))
    }

    pub fn default_model(&self) -> &str {
        &self.standard.primary
    }

    pub fn default_agent(&self, models: &ModelRoster) -> Result<String> {
        Ok(exact_spec(self.default_model(), models)?.canonical())
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

    pub fn allowed_agents(&self) -> String {
        self.agents().collect::<Vec<_>>().join(", ")
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
impl From<String> for RoleBinding {
    fn from(primary: String) -> Self {
        Self {
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
