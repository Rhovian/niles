use serde::{Deserialize, Serialize};

/// Which agent plays each role in this workspace.
///
/// Every role that has its own brief has its own binding, `security` included: it is
/// commissioned rarely, but when it is, the tier it runs at is a workspace decision rather
/// than something the lead should have to remember per spawn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "WorkspaceManifestWire")]
pub struct WorkspaceManifest {
    pub lead: String,
    pub worker: String,
    pub reviewer: String,
    pub security: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceManifestWire {
    lead: String,
    worker: String,
    reviewer: String,
    security: String,
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
        }
    }
}
