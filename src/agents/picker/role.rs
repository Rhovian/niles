use crate::workspace_manifest::{RoleBinding, WorkspaceManifest};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Lead,
    Worker,
    Reviewer,
    Security,
    Design,
}

impl Role {
    pub(crate) const ALL: [Self; 5] = [
        Self::Lead,
        Self::Worker,
        Self::Reviewer,
        Self::Security,
        Self::Design,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Worker => "worker",
            Self::Reviewer => "reviewer",
            Self::Security => "security",
            Self::Design => "design",
        }
    }

    fn binding(self, manifest: &WorkspaceManifest) -> Option<&RoleBinding> {
        match self {
            Self::Lead => None,
            Self::Worker => Some(&manifest.worker),
            Self::Reviewer => Some(&manifest.reviewer),
            Self::Security => Some(&manifest.security),
            Self::Design => Some(&manifest.design),
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
            Self::Reviewer => binding_value(&manifest.reviewer),
            Self::Security => binding_value(&manifest.security),
            Self::Design => binding_value(&manifest.design),
        }
    }

    pub(crate) fn scalar(self) -> Option<ScalarRole> {
        match self {
            Self::Lead => Some(ScalarRole::Lead),
            Self::Worker => Some(ScalarRole::Worker),
            Self::Reviewer => Some(ScalarRole::Reviewer),
            Self::Security => Some(ScalarRole::Security),
            Self::Design => None,
        }
    }

    pub(crate) fn changed(self, before: &WorkspaceManifest, after: &WorkspaceManifest) -> bool {
        match self {
            Self::Lead => before.lead != after.lead,
            Self::Worker => before.worker != after.worker,
            Self::Reviewer => before.reviewer != after.reviewer,
            Self::Security => before.security != after.security,
            Self::Design => before.design != after.design,
        }
    }
}

fn binding_value(binding: &RoleBinding) -> String {
    match binding.scalar() {
        Some(value) => value,
        None => binding.default_model().to_owned(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScalarRole {
    Lead,
    Worker,
    Reviewer,
    Security,
}

impl ScalarRole {
    pub(crate) fn set(self, manifest: &mut WorkspaceManifest, value: String) {
        match self {
            Self::Lead => manifest.lead = value,
            Self::Worker => manifest.worker = value.into(),
            Self::Reviewer => manifest.reviewer = value.into(),
            Self::Security => manifest.security = value.into(),
        }
    }
}

impl From<ScalarRole> for Role {
    fn from(role: ScalarRole) -> Self {
        match role {
            ScalarRole::Lead => Self::Lead,
            ScalarRole::Worker => Self::Worker,
            ScalarRole::Reviewer => Self::Reviewer,
            ScalarRole::Security => Self::Security,
        }
    }
}
