use serde::{Deserialize, Serialize};

/// Which agent plays each role in this workspace, and the check-in cadence a dispatch arms.
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
    pub reviewer: String,
    pub security: String,
    /// `checkin:` — the delay `spawn` and `send` arm when `--checkin` is not given: `15m`, `90s`,
    /// a bare number of minutes, or `off` for none. Absent is the built-in five-minute default.
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceManifestWire {
    lead: String,
    worker: String,
    reviewer: String,
    security: String,
    #[serde(default)]
    checkin: Option<String>,
    #[serde(default)]
    recheck: Option<String>,
    #[serde(default, rename = "niles_schema")]
    _niles_schema: Option<u64>,
}

impl From<WorkspaceManifestWire> for WorkspaceManifest {
    fn from(wire: WorkspaceManifestWire) -> Self {
        Self {
            lead: wire.lead,
            worker: wire.worker,
            reviewer: wire.reviewer,
            security: wire.security,
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
            reviewer: "claude".to_owned(),
            security: "claude".to_owned(),
            checkin: None,
            recheck: None,
        }
    }
}
