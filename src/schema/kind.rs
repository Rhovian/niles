#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ArtifactKind {
    Directory,
    ManagerSession,
    WorkerMetadata,
    WorkspaceManifest,
}

impl ArtifactKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ArtifactKind::Directory => "artifact directory",
            ArtifactKind::ManagerSession => "manager session metadata",
            ArtifactKind::WorkerMetadata => "worker metadata",
            ArtifactKind::WorkspaceManifest => "workspace manifest",
        }
    }

    pub(in crate::schema) fn remediation(self) -> &'static str {
        match self {
            ArtifactKind::Directory => "fix the directory permissions and rerun `niles doctor`",
            ArtifactKind::ManagerSession => {
                "remove the session directory and start a fresh manager session, or use the older binary that wrote it"
            }
            ArtifactKind::WorkerMetadata => {
                "remove the worker dir and respawn, or use the older binary to close it"
            }
            ArtifactKind::WorkspaceManifest => {
                "delete .niles/manifest.yaml and rerun `niles`, or use the older binary that wrote it"
            }
        }
    }
}
