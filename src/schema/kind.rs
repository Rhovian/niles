#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ArtifactKind {
    CapabilityManifest,
    Directory,
    GlobalIndex,
    ManagerSession,
    UsageSnapshot,
    WorkerMetadata,
    WorkerPointer,
    WorkspaceManifest,
    WorkspaceTmuxSession,
}

impl ArtifactKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ArtifactKind::CapabilityManifest => "capability manifest",
            ArtifactKind::Directory => "artifact directory",
            ArtifactKind::GlobalIndex => "global Niles index",
            ArtifactKind::ManagerSession => "manager session metadata",
            ArtifactKind::UsageSnapshot => "usage snapshot",
            ArtifactKind::WorkerMetadata => "worker metadata",
            ArtifactKind::WorkerPointer => "worker pointer",
            ArtifactKind::WorkspaceManifest => "workspace manifest",
            ArtifactKind::WorkspaceTmuxSession => "workspace tmux session pointer",
        }
    }

    pub(in crate::schema) fn remediation(self) -> &'static str {
        match self {
            ArtifactKind::CapabilityManifest => {
                "rerun `niles analyze` to regenerate it, or use the older binary that wrote it"
            }
            ArtifactKind::Directory => "fix the directory permissions and rerun `niles doctor`",
            ArtifactKind::GlobalIndex => {
                "use the binary that wrote it, or remove the index only if you accept losing existing cross-workspace worker pointers"
            }
            ArtifactKind::ManagerSession => {
                "remove the session directory and start a fresh manager session, or use the older binary that wrote it"
            }
            ArtifactKind::UsageSnapshot => {
                "remove the usage snapshot and recapture usage if the source transcript is still available, or use the older binary that wrote it"
            }
            ArtifactKind::WorkerMetadata => {
                "remove the worker dir and respawn, or use the older binary to close it"
            }
            ArtifactKind::WorkerPointer => {
                "remove the pointer file and respawn the worker, or use the older binary that wrote it"
            }
            ArtifactKind::WorkspaceManifest => {
                "delete .niles/manifest.yaml and rerun `niles`, or use the older binary that wrote it"
            }
            ArtifactKind::WorkspaceTmuxSession => {
                "remove .niles/sessions/tmux-session.json and respawn the worker, or use the older binary that wrote it"
            }
        }
    }
}
