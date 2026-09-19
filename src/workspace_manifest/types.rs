use serde::{Deserialize, Serialize};

/// Which agent plays each standing role in this workspace.
///
/// A security pass uses the `reviewer` binding unless the lead names an agent for it; it is
/// commissioned rarely and deliberately, so a fourth binding would mostly go stale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "WorkspaceManifestWire")]
pub struct WorkspaceManifest {
    pub lead: String,
    pub worker: String,
    pub reviewer: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceManifestWire {
    lead: String,
    worker: String,
    reviewer: String,
    #[serde(default, rename = "niles_schema")]
    _niles_schema: Option<u64>,
}

impl From<WorkspaceManifestWire> for WorkspaceManifest {
    fn from(wire: WorkspaceManifestWire) -> Self {
        Self {
            lead: wire.lead,
            worker: wire.worker,
            reviewer: wire.reviewer,
        }
    }
}

impl Default for WorkspaceManifest {
    fn default() -> Self {
        Self {
            lead: "claude".to_owned(),
            worker: "codex".to_owned(),
            reviewer: "claude".to_owned(),
        }
    }
}
