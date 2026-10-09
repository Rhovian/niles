use std::collections::BTreeSet;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize, Serializer};

use crate::agents::{AgentSpec, ModelRoster, roster};

/// Which agent plays each role, plus operator-authored planning and check-in settings.
///
/// Every role that has its own brief has its own binding, `security` included: it is
/// commissioned rarely, but when it is, the tier it runs at is a workspace decision rather
/// than something the lead should have to remember per spawn.
///
/// The check-in keys are workspace-wide rather than per role: they say how often the lead is
/// nudged about a worker that has gone quiet, which is a property of the workspace's pace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "WorkspaceManifestWire")]
pub struct WorkspaceManifest {
    pub lead: String,
    pub worker: RoleBinding,
    pub reviewer: ReviewerBinding,
    pub security: RoleBinding,
    /// Planning guidance for exact `family:model` pairs. The lead consults this only for
    /// implementation assignments; Niles does not interpret models or infer capabilities.
    #[serde(default, skip_serializing_if = "WorkerPlanning::is_empty")]
    pub worker_planning: WorkerPlanning,
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
pub enum ReviewerBinding {
    #[serde(rename = "lead")]
    Lead,
    #[serde(untagged)]
    Agent(RoleBinding),
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

/// Guidance groups in which each `family:model` appears at most once, so a model never
/// carries two instructions the lead would have to choose between.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<PlanningGroup>")]
pub struct WorkerPlanning(pub Vec<PlanningGroup>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningGroup {
    pub models: Vec<String>,
    pub guidance: String,
}

impl WorkerPlanning {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl TryFrom<Vec<PlanningGroup>> for WorkerPlanning {
    type Error = String;

    fn try_from(groups: Vec<PlanningGroup>) -> std::result::Result<Self, Self::Error> {
        let mut seen = BTreeSet::new();
        for model in groups.iter().flat_map(|group| &group.models) {
            if !seen.insert(model) {
                return Err(format!(
                    "{model} appears in more than one worker_planning group"
                ));
            }
        }
        Ok(Self(groups))
    }
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

impl From<String> for RoleBinding {
    fn from(value: String) -> Self {
        let (model, effort) = match value.match_indices(':').nth(1) {
            Some((index, _)) => (&value[..index], Some(&value[index + 1..])),
            None => (value.as_str(), None),
        };
        Self(vec![AgentGroup {
            when: None,
            models: vec![model.to_owned()],
            efforts: effort.map(|effort| vec![effort.to_owned()]),
        }])
    }
}

fn listed_spec(model: &str, roster: &ModelRoster) -> Result<AgentSpec> {
    let spec = AgentSpec::parse(model, roster)?;
    if spec.effort().is_some() {
        bail!("listed model `{model}` must not carry an effort; list it under efforts");
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
        let spec = listed_spec(&group.models[0], models)?;
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
                let listed = listed_spec(model, models)?;
                if listed.family() != requested.family() || listed.model() != requested.model() {
                    continue;
                }
                let allowed = match (&group.efforts, requested.effort()) {
                    (None, _) => true,
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

pub(crate) const DEFAULT_REVIEWER_AGENT: &str = "claude";

impl ReviewerBinding {
    pub fn as_agent(&self) -> Option<&RoleBinding> {
        match self {
            Self::Lead => None,
            Self::Agent(agent) => Some(agent),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceManifestWire {
    lead: String,
    worker: RoleBinding,
    reviewer: ReviewerBinding,
    security: RoleBinding,
    #[serde(default)]
    worker_planning: WorkerPlanning,
    #[serde(default)]
    checkin: Option<String>,
    #[serde(default)]
    recheck: Option<String>,
}

impl From<WorkspaceManifestWire> for WorkspaceManifest {
    fn from(wire: WorkspaceManifestWire) -> Self {
        Self {
            lead: wire.lead,
            worker: wire.worker,
            reviewer: wire.reviewer,
            security: wire.security,
            worker_planning: wire.worker_planning,
            checkin: wire.checkin,
            recheck: wire.recheck,
        }
    }
}

impl Default for WorkspaceManifest {
    fn default() -> Self {
        Self {
            lead: "claude".to_owned(),
            worker: "codex".to_owned().into(),
            reviewer: ReviewerBinding::Agent(DEFAULT_REVIEWER_AGENT.to_owned().into()),
            security: "claude".to_owned().into(),
            worker_planning: WorkerPlanning::default(),
            checkin: None,
            recheck: None,
        }
    }
}
