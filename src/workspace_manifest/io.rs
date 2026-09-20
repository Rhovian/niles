use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    schema::{self, ArtifactKind},
    store::paths::NILES_DIR,
};

use super::WorkspaceManifest;

pub fn manifest_path(root: &Utf8Path) -> Utf8PathBuf {
    root.join(NILES_DIR).join("manifest.yaml")
}

pub fn load(root: &Utf8Path) -> Result<Option<WorkspaceManifest>> {
    let path = manifest_path(root);
    schema::read_optional_yaml(&path, ArtifactKind::WorkspaceManifest)
}

pub fn save(root: &Utf8Path, manifest: &WorkspaceManifest) -> Result<()> {
    let path = manifest_path(root);
    let parent = path
        .parent()
        .with_context(|| format!("workspace manifest path has no parent: {path}"))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    schema::write_yaml(&path, manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use super::super::{test_support::temp_test_path, types::WorkspaceManifest};

    #[test]
    fn skewed_manifest_remediation_names_delete_and_rerun() {
        let root = temp_test_path("skewed-remediation");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(manifest_path(&root), "lead: codex\n").unwrap();

        let err = load(&root).unwrap_err().to_string();

        assert!(err.contains("workspace manifest"));
        assert!(err.contains("schema 1"));
        assert!(err.contains("delete .niles/manifest.yaml and rerun `niles`"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn current_manifest_loads_role_bindings() {
        let root = temp_test_path("manifest-roles");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(
            manifest_path(&root),
            r#"
lead: claude
worker: codex
reviewer: claude
security: claude
niles_schema: 2
"#,
        )
        .unwrap();

        let manifest = load(&root).unwrap().unwrap();

        assert_eq!(
            manifest,
            WorkspaceManifest {
                lead: "claude".to_owned(),
                worker: "codex".to_owned(),
                reviewer: "claude".to_owned(),
                security: "claude".to_owned(),
            }
        );

        fs::remove_dir_all(root).unwrap();
    }

    /// The manifest carries role bindings and nothing else, so a manifest written before the
    /// lead rename is rejected by name rather than silently losing its fields.
    #[test]
    fn pre_lead_manifest_is_rejected_and_names_the_offending_field() {
        let root = temp_test_path("manifest-pre-lead");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(
            manifest_path(&root),
            r#"
manager: claude
planner: claude
worker: codex
reviewer: claude
validation_command: test
flow:
  - planner
  - worker
  - reviewer
niles_schema: 2
"#,
        )
        .unwrap();

        let err = format!("{:#}", load(&root).unwrap_err());

        // Names the offending field, the fields that replaced it, and what to do about it.
        assert!(err.contains("unknown field `manager`"), "{err}");
        assert!(
            err.contains("expected one of `lead`, `worker`, `reviewer`, `security`"),
            "{err}"
        );
        assert!(
            err.contains("delete .niles/manifest.yaml and rerun `niles`"),
            "{err}"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saving_manifest_writes_role_bindings_and_nothing_else() {
        let root = temp_test_path("manifest-save");
        let manifest = WorkspaceManifest {
            lead: "claude".to_owned(),
            worker: "codebot".to_owned(),
            reviewer: "reviewbot".to_owned(),
            security: "auditbot".to_owned(),
        };

        save(&root, &manifest).unwrap();
        let body = fs::read_to_string(manifest_path(&root)).unwrap();

        assert!(body.contains("lead: claude"), "{body}");
        assert!(body.contains("worker: codebot"), "{body}");
        assert!(body.contains("reviewer: reviewbot"), "{body}");
        assert!(body.contains("security: auditbot"), "{body}");
        for gone in ["manager:", "planner:", "validation_command:", "flow:"] {
            assert!(!body.contains(gone), "{gone} should be gone:\n{body}");
        }

        fs::remove_dir_all(root).unwrap();
    }
}
