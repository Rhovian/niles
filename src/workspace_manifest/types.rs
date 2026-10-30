use std::{
    collections::{BTreeMap, HashSet},
    time::Duration,
};

use crate::skills::SkillName;
use anyhow::{Context, Result, bail, ensure};
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
    pub gates: Option<GateConfig>,
    pub standard: TierBinding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontier: Option<TierBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub economy: Option<TierBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mechanical: Option<TierBinding>,
}

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawGates<Duration>")]
pub struct Gates {
    pub rounds: Rounds,
    pub post: Vec<RunGate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawGates<String>", into = "RawGates<String>")]
pub struct GateConfig(pub Gates);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunGate {
    pub name: SkillName,
    pub run: Vec<String>,
    pub timeout: Duration,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "T: Deserialize<'de>"))]
struct RawGates<T> {
    #[serde(default)]
    rounds: Rounds,
    #[serde(default)]
    post: Vec<RawGate<T>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGate<T> {
    name: Option<SkillName>,
    run: Vec<String>,
    timeout: Option<T>,
}

impl TryFrom<RawGates<Duration>> for Gates {
    type Error = anyhow::Error;
    fn try_from(raw: RawGates<Duration>) -> Result<Self> {
        raw.parse(Ok)
    }
}

impl TryFrom<RawGates<String>> for GateConfig {
    type Error = anyhow::Error;
    fn try_from(raw: RawGates<String>) -> Result<Self> {
        raw.parse(|value| crate::duration::parse_duration(&value))
            .map(Self)
    }
}

impl From<GateConfig> for RawGates<String> {
    fn from(config: GateConfig) -> Self {
        Self {
            rounds: config.0.rounds,
            post: config
                .0
                .post
                .into_iter()
                .map(|gate| RawGate {
                    name: Some(gate.name),
                    run: gate.run,
                    timeout: Some(format!("{}ms", gate.timeout.as_millis())),
                })
                .collect(),
        }
    }
}

// serde 1.0.229's try_from delegates here; serde-saphyr 1.3.0 does not
// enforce argv, identifier uniqueness or the list's cumulative time budget.
impl<T> RawGates<T> {
    fn parse(self, parse_timeout: impl Fn(T) -> Result<Duration>) -> Result<Gates> {
        let mut names = HashSet::new();
        let mut total = Duration::ZERO;
        let mut post = Vec::new();
        for (index, gate) in self.post.into_iter().enumerate() {
            let name = match gate.name {
                Some(name) => name,
                None => format!("run-{}", index + 1).parse()?,
            };
            ensure!(names.insert(name.clone()), "duplicate gate name: {name}");
            ensure!(
                !gate.run.is_empty(),
                "gate {name}: run argv must not be empty"
            );
            let timeout = match gate.timeout {
                Some(value) => parse_timeout(value)?,
                None => DEFAULT_TIMEOUT,
            };
            ensure!(!timeout.is_zero(), "gate timeout must be greater than zero");
            total += timeout;
            ensure!(total <= MAX_TIMEOUT, "gate timeout sum exceeds 10 minutes");
            post.push(RunGate {
                name,
                run: gate.run,
                timeout,
            });
        }
        Ok(Gates {
            rounds: self.rounds,
            post,
        })
    }
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
    pub(crate) fn attached_skills(&self) -> impl Iterator<Item = &SkillName> {
        self.skills
            .iter()
            .chain(&self.always)
            .chain(&self.on_from)
            .chain(&self.on_stage)
    }

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

impl From<TierBinding> for Binding {
    fn from(standard: TierBinding) -> Self {
        Self {
            skills: Vec::new(),
            always: Vec::new(),
            on_from: Vec::new(),
            on_stage: Vec::new(),
            gates: None,
            standard,
            frontier: None,
            economy: None,
            mechanical: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl From<String> for Binding {
        fn from(primary: String) -> Self {
            Self::from(TierBinding {
                primary,
                secondaries: Vec::new(),
            })
        }
    }

    #[test]
    fn rounds_are_bounded_at_yaml_and_json_parse_boundaries() {
        for value in [0, 11, 256, -1] {
            let body = format!("rounds: {value}");
            assert!(
                crate::store::parse_yaml::<GateConfig>(&body).is_err(),
                "{body}"
            );
            assert!(serde_json::from_value::<Gates>(serde_json::json!({"rounds": value})).is_err());
        }
        for value in [1, 10] {
            let gates: GateConfig = crate::store::parse_yaml(&format!("rounds: {value}")).unwrap();
            assert_eq!(u8::from(gates.0.rounds), value);
        }
        let gates: GateConfig = crate::store::parse_yaml("{}").unwrap();
        assert_eq!(u8::from(gates.0.rounds), 1);
        assert!(crate::store::parse_yaml::<GateConfig>("typo: 1").is_err());
    }

    #[test]
    fn gates_normalize_and_reject_invalid_lists() {
        let gates: GateConfig =
            crate::store::parse_yaml("post: [{run: [true]}, {run: [false]}]").unwrap();
        assert_eq!(gates.0.post[0].timeout, Duration::from_secs(300));
        for body in [
            "post: [{run: []}]",
            "post: [{run: [true], timeout: 601s}]",
            "post: [{run: [true], timeout: 6m}, {run: [true], timeout: 5m}]",
            "post: [{name: same, run: [true]}, {name: same, run: [true]}]",
            "post: [{run: [true]}, {name: run-1, run: [true]}]",
            "post: [{name: '../escape', run: [true]}]",
            "post: [{agent: reviewer}]",
            "post: [{run: [true], typo: value}]",
        ] {
            assert!(
                crate::store::parse_yaml::<GateConfig>(body).is_err(),
                "{body}"
            );
        }
        assert!(
            crate::store::parse_yaml::<GateConfig>("post: [{run: [true], timeout: 10m}]").is_ok()
        );
    }

    #[test]
    fn zero_gate_timeouts_fail_at_yaml_and_json_boundaries() {
        let message = "gate timeout must be greater than zero";
        for timeout in ["0", "0ms"] {
            let body = format!("post: [{{run: [true], timeout: {timeout}}}]");
            let error = crate::store::parse_yaml::<GateConfig>(&body).unwrap_err();
            assert!(error.to_string().contains(message));
            let value = serde_json::json!({"post": [{"run": ["true"], "timeout": timeout}]});
            let error = serde_json::from_value::<GateConfig>(value).unwrap_err();
            assert!(error.to_string().contains(message));
        }
        let value = serde_json::json!({"post": [{"run": ["true"], "timeout": Duration::ZERO}]});
        let error = serde_json::from_value::<Gates>(value).unwrap_err();
        assert!(error.to_string().contains(message));
    }
}
