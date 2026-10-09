use crate::workspace_manifest::{ReviewerBinding, RoleBinding, WorkspaceManifest};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Lead,
    Worker,
    Reviewer,
    Security,
}

impl Role {
    pub(crate) const ALL: [Self; 4] = [Self::Lead, Self::Worker, Self::Reviewer, Self::Security];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Worker => "worker",
            Self::Reviewer => "reviewer",
            Self::Security => "security",
        }
    }

    fn binding(self, manifest: &WorkspaceManifest) -> Option<&RoleBinding> {
        match self {
            Self::Lead => None,
            Self::Worker => Some(&manifest.worker),
            Self::Reviewer => manifest.reviewer.as_agent(),
            Self::Security => Some(&manifest.security),
        }
    }

    pub(crate) fn groups(self, manifest: &WorkspaceManifest) -> Option<usize> {
        self.binding(manifest)
            .filter(|binding| binding.scalar().is_none())
            .map(|binding| binding.0.len())
    }

    pub(crate) fn value(self, manifest: &WorkspaceManifest) -> String {
        match self {
            Self::Lead => manifest.lead.clone(),
            Self::Worker => binding_value(&manifest.worker),
            Self::Reviewer => match &manifest.reviewer {
                ReviewerBinding::Lead => "lead".to_owned(),
                ReviewerBinding::Agent(binding) => binding_value(binding),
            },
            Self::Security => binding_value(&manifest.security),
        }
    }

    pub(crate) fn set(self, manifest: &mut WorkspaceManifest, value: String) {
        match self {
            Self::Lead => manifest.lead = value,
            Self::Worker => manifest.worker = value.into(),
            Self::Reviewer => {
                manifest.reviewer = if value == "lead" {
                    ReviewerBinding::Lead
                } else {
                    ReviewerBinding::Agent(value.into())
                }
            }
            Self::Security => manifest.security = value.into(),
        }
    }

    pub(crate) fn changed(self, before: &WorkspaceManifest, after: &WorkspaceManifest) -> bool {
        match self {
            Self::Lead => before.lead != after.lead,
            Self::Worker => before.worker != after.worker,
            Self::Reviewer => before.reviewer != after.reviewer,
            Self::Security => before.security != after.security,
        }
    }
}

fn binding_value(binding: &RoleBinding) -> String {
    match binding.scalar() {
        Some(value) => value,
        None => binding.default_model().to_owned(),
    }
}
