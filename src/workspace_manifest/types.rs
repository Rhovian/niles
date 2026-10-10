use std::collections::BTreeSet;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize, Serializer};

use crate::agents::{AgentSpec, ModelRoster, profile_for, roster};

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
    pub lead: String,
    pub worker: RoleBinding,
    #[serde(deserialize_with = "reviewer_binding")]
    pub reviewer: RoleBinding,
    pub security: RoleBinding,
    #[serde(deserialize_with = "design_binding")]
    pub design: RoleBinding,
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "RoleBindingWire")]
pub struct RoleBinding(pub Vec<AgentGroup>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    pub models: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub efforts: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RoleBindingWire {
    Scalar(String),
    Groups(Vec<AgentGroup>),
}

impl TryFrom<RoleBindingWire> for RoleBinding {
    type Error = String;

    fn try_from(wire: RoleBindingWire) -> std::result::Result<Self, Self::Error> {
        let groups = match wire {
            RoleBindingWire::Scalar(value) => return Ok(Self::from(value)),
            RoleBindingWire::Groups(groups) => groups,
        };
        if groups.is_empty()
            || groups.iter().any(|group| {
                group.models.is_empty() || group.efforts.as_ref().is_some_and(Vec::is_empty)
            })
        {
            return Err("a role needs at least one group, each with models, and efforts non-empty when present".into());
        }
        Ok(Self(groups))
    }
}

impl Serialize for RoleBinding {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self.scalar() {
            Some(value) => serializer.serialize_str(&value),
            None => self.0.serialize(serializer),
        }
    }
}

/// The value stays whole: only the roster can tell an effort from a model name that itself
/// contains `:` (OpenRouter's `:free` variants), so the effort is read when the spec is parsed.
impl From<String> for RoleBinding {
    fn from(value: String) -> Self {
        Self(vec![AgentGroup {
            when: None,
            models: vec![value],
            efforts: None,
        }])
    }
}

fn listed_spec(model: &str, group: &AgentGroup, roster: &ModelRoster) -> Result<AgentSpec> {
    let spec = AgentSpec::parse(model, roster)?;
    if spec.effort().is_some() && group.efforts.is_some() {
        bail!("listed model `{model}` must not carry an effort alongside an efforts list");
    }
    Ok(spec)
}

impl RoleBinding {
    /// The binding as one `model[:effort]` value, unless it carries groups, `when` text or an effort
    /// list that only the list form can hold.
    pub fn scalar(&self) -> Option<String> {
        let [
            AgentGroup {
                when: None,
                models,
                efforts,
            },
        ] = self.0.as_slice()
        else {
            return None;
        };
        let [model] = models.as_slice() else {
            return None;
        };
        match efforts.as_deref() {
            None => Some(model.clone()),
            Some([effort]) => Some(format!("{model}:{effort}")),
            Some(_) => None,
        }
    }

    pub fn default_model(&self) -> &str {
        &self.0[0].models[0]
    }

    pub fn default_agent(&self, models: &ModelRoster) -> Result<String> {
        let group = &self.0[0];
        let spec = listed_spec(&group.models[0], group, models)?;
        let (Some(model), Some(efforts)) = (spec.model(), &group.efforts) else {
            return Ok(spec.canonical());
        };
        let Some(supported) = models
            .supported_efforts(spec.family(), model)
            .filter(|supported| !supported.is_empty())
        else {
            return Ok(spec.canonical());
        };
        let effort = roster::normalize_effort(&efforts[0]);
        if supported.contains(&effort) {
            return Ok(format!("{}:{effort}", spec.canonical()));
        }
        bail!(
            "model {model} does not support listed effort {effort}; listed efforts: {}",
            efforts.join(", ")
        )
    }

    pub fn allows(&self, requested: &AgentSpec, models: &ModelRoster) -> Result<bool> {
        for group in &self.0 {
            for model in &group.models {
                let listed = listed_spec(model, group, models)?;
                if listed.family() != requested.family() || listed.model() != requested.model() {
                    continue;
                }
                let allowed = match (&group.efforts, requested.effort()) {
                    (None, _) => listed
                        .effort()
                        .is_none_or(|effort| requested.effort() == Some(effort)),
                    (Some(efforts), Some(effort)) => efforts
                        .iter()
                        .any(|listed| roster::normalize_effort(listed) == effort),
                    (Some(_), None) => listed.model().is_none_or(|model| {
                        models
                            .supported_efforts(listed.family(), model)
                            .is_some_and(<[String]>::is_empty)
                    }),
                };
                if allowed {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    pub fn agents(&self) -> impl Iterator<Item = &str> {
        self.0
            .iter()
            .flat_map(|group| group.models.iter().map(String::as_str))
    }

    pub fn allowed_agents(&self) -> String {
        self.0
            .iter()
            .map(|group| {
                let models = group.models.join(", ");
                match &group.efforts {
                    Some(efforts) => format!("{models} [{}]", efforts.join(", ")),
                    None => models,
                }
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

fn design_binding<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<RoleBinding, D::Error> {
    let binding = RoleBinding::deserialize(deserializer)?;
    let families: BTreeSet<_> = binding
        .agents()
        .map(|agent| {
            let family = agent.split_once(':').map_or(agent, |(family, _)| family);
            profile_for(family).map_or(family, |profile| profile.id)
        })
        .collect();
    if families.len() < 2 {
        return Err(serde::de::Error::custom(format!(
            "design must list models from at least two agent families; found: {}",
            families.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    Ok(binding)
}

fn reviewer_binding<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<RoleBinding, D::Error> {
    let binding = RoleBinding::deserialize(deserializer)?;
    if binding.scalar().as_deref() == Some("lead") {
        return Err(serde::de::Error::custom(
            "reviewer: lead is no longer supported; reviewer must be an agent binding",
        ));
    }
    Ok(binding)
}

impl Default for WorkspaceManifest {
    fn default() -> Self {
        Self {
            lead: "claude".to_owned(),
            worker: "codex".to_owned().into(),
            reviewer: "claude".to_owned().into(),
            security: "claude".to_owned().into(),
            design: RoleBinding(vec![AgentGroup {
                when: None,
                models: vec!["claude".into(), "codex".into()],
                efforts: None,
            }]),
            checkin: None,
            recheck: None,
        }
    }
}
