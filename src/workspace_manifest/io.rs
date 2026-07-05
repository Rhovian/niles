use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::schema::{self, ArtifactKind};

use super::WorkspaceManifest;

const MANIFEST_RELATIVE_PATH: &str = ".niles/manifest.yaml";

pub fn manifest_path(root: &Utf8Path) -> Utf8PathBuf {
    root.join(MANIFEST_RELATIVE_PATH)
}

pub fn load(root: &Utf8Path) -> Result<Option<WorkspaceManifest>> {
    let path = manifest_path(root);
    schema::read_optional_yaml(&path, ArtifactKind::WorkspaceManifest)
}

pub fn load_required(root: &Utf8Path) -> Result<WorkspaceManifest> {
    load(root)?.with_context(|| {
        format!(
            "workspace manifest {} does not exist; run `niles` in an interactive tmux session to configure workspace roles",
            manifest_path(root)
        )
    })
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

    use super::super::{
        test_support::temp_test_path,
        types::{flow_summary, WorkspaceFlowRole, WorkspaceManifest, WORKER_REVIEW_LOOP_SUMMARY},
    };

    #[test]
    fn skewed_manifest_remediation_names_delete_and_rerun() {
        let root = temp_test_path("skewed-remediation");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(manifest_path(&root), "manager: codex\n").unwrap();

        let err = load(&root).unwrap_err().to_string();

        assert!(err.contains("workspace manifest"));
        assert!(err.contains("schema 1"));
        assert!(err.contains("delete .niles/manifest.yaml and rerun `niles`"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manifest_without_flow_loads_initial_flow() {
        let root = temp_test_path("manifest-flow");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(
            manifest_path(&root),
            r#"
manager: codex
planner: claude
worker: codex
reviewer: claude
validation_command: lint
niles_schema: 2
"#,
        )
        .unwrap();

        let manifest = load(&root).unwrap().unwrap();

        assert_eq!(flow_summary(&manifest.flow), WORKER_REVIEW_LOOP_SUMMARY);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn current_manifest_loads_flow() {
        let root = temp_test_path("manifest-current-flow");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(
            manifest_path(&root),
            r#"
manager: codex
planner: claude
worker: codex
reviewer: claude
validation_command: lint
flow:
  - planner
  - reviewer
niles_schema: 2
"#,
        )
        .unwrap();

        let manifest = load(&root).unwrap().unwrap();

        assert_eq!(flow_summary(&manifest.flow), "planner -> reviewer");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saving_manifest_writes_flow() {
        let root = temp_test_path("manifest-save-flow");
        let manifest = WorkspaceManifest {
            manager: "claude".to_owned(),
            planner: "planbot".to_owned(),
            worker: "codebot".to_owned(),
            reviewer: "reviewbot".to_owned(),
            validation_command: "check".to_owned(),
            flow: vec![WorkspaceFlowRole::Worker],
        };

        save(&root, &manifest).unwrap();
        let body = fs::read_to_string(manifest_path(&root)).unwrap();

        assert!(body.contains("flow:\n- worker"));
        fs::remove_dir_all(root).unwrap();
    }
}
