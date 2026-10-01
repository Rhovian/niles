use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
    pub worker: String,
    pub reviewer: ReviewerBinding,
    pub security: String,
    /// Planning guidance keyed by an exact `family:model` pair. The lead consults this only for
    /// implementation assignments; Niles does not interpret models or infer capabilities.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub worker_planning: BTreeMap<String, String>,
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
    Agent(String),
}

pub(crate) const DEFAULT_REVIEWER_AGENT: &str = "claude";

impl ReviewerBinding {
    pub fn as_agent(&self) -> Option<&str> {
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
    worker: String,
    reviewer: ReviewerBinding,
    security: String,
    #[serde(default)]
    worker_planning: BTreeMap<String, String>,
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
            worker: "codex".to_owned(),
            reviewer: ReviewerBinding::Agent(DEFAULT_REVIEWER_AGENT.to_owned()),
            security: "claude".to_owned(),
            worker_planning: BTreeMap::new(),
            checkin: None,
            recheck: None,
        }
    }
}
