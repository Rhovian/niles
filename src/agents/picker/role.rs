use crate::workspace_manifest::{RoleBinding, WorkspaceManifest};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Lead,
    Worker,
    Reviewer,
    Security,
    Design,
    Research,
}

impl Role {
    pub(crate) const ALL: [Self; 6] = [
        Self::Lead,
        Self::Worker,
        Self::Reviewer,
        Self::Security,
        Self::Design,
        Self::Research,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Worker => "worker",
            Self::Reviewer => "reviewer",
            Self::Security => "security",
            Self::Design => "design",
            Self::Research => "research",
        }
    }

    pub(crate) fn binding(self, manifest: &WorkspaceManifest) -> &RoleBinding {
        match self {
            Self::Lead => &manifest.lead,
            Self::Worker => &manifest.worker,
            Self::Reviewer => &manifest.reviewer,
            Self::Security => &manifest.security,
            Self::Design => &manifest.design,
            Self::Research => &manifest.research,
        }
    }

    pub(crate) fn value(self, manifest: &WorkspaceManifest) -> String {
        self.binding(manifest).standard.primary.clone()
    }

    pub(crate) fn changed(self, before: &WorkspaceManifest, after: &WorkspaceManifest) -> bool {
        match self {
            Self::Lead => before.lead != after.lead,
            Self::Worker => before.worker != after.worker,
            Self::Reviewer => before.reviewer != after.reviewer,
            Self::Security => before.security != after.security,
            Self::Design => before.design != after.design,
            Self::Research => before.research != after.research,
        }
    }

    pub(crate) fn set(self, manifest: &mut WorkspaceManifest, value: String) {
        let binding = match self {
            Self::Lead => &mut manifest.lead,
            Self::Worker => &mut manifest.worker,
            Self::Reviewer => &mut manifest.reviewer,
            Self::Security => &mut manifest.security,
            Self::Design => &mut manifest.design,
            Self::Research => &mut manifest.research,
        };
        binding.standard.primary = value;
    }
}
