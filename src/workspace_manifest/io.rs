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
                ..WorkspaceManifest::default()
            }
        );

        fs::remove_dir_all(root).unwrap();
    }

    /// The two check-in keys are read from the manifest as written, and a manifest that says
    /// nothing about them carries no default of its own: the cadence resolver owns that decision.
    #[test]
    fn manifest_check_in_keys_load_and_absent_ones_stay_absent() {
        let root = temp_test_path("manifest-checkin");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(
            manifest_path(&root),
            r#"
lead: claude
worker: codex
reviewer: claude
security: claude
checkin: 15m
recheck: backoff
niles_schema: 2
"#,
        )
        .unwrap();
        let anonymous = temp_test_path("manifest-no-checkin");
        fs::create_dir_all(anonymous.join(".niles")).unwrap();
        fs::write(
            manifest_path(&anonymous),
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
        let bare = load(&anonymous).unwrap().unwrap();

        assert_eq!(manifest.checkin.as_deref(), Some("15m"));
        assert_eq!(manifest.recheck.as_deref(), Some("backoff"));
        assert_eq!(bare.checkin, None);
        assert_eq!(bare.recheck, None);
        assert!(bare.worker_planning.is_empty());

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(anonymous).unwrap();
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
            err.contains(
                "unknown field `manager`, expected one of lead, worker, reviewer, security, worker_planning, checkin, recheck, niles_schema"
            ),
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
            ..WorkspaceManifest::default()
        };

        save(&root, &manifest).unwrap();
        let body = fs::read_to_string(manifest_path(&root)).unwrap();

        assert!(body.contains("lead: claude"), "{body}");
        assert!(body.contains("worker: codebot"), "{body}");
        assert!(body.contains("reviewer: reviewbot"), "{body}");
        assert!(body.contains("security: auditbot"), "{body}");
        for gone in [
            "manager:",
            "planner:",
            "validation_command:",
            "flow:",
            // A manifest that says nothing about the check-in cadence is written back saying
            // nothing: `checkin: null` would be a value the next reader has to interpret.
            "checkin:",
            "recheck:",
            "worker_planning:",
        ] {
            assert!(!body.contains(gone), "{gone} should be gone:\n{body}");
        }

        // And one that does carry them keeps them, so a round trip through `save` is not a
        // silent reset to the built-in cadence.
        let configured = WorkspaceManifest {
            checkin: Some("15m".to_owned()),
            recheck: Some("backoff".to_owned()),
            worker_planning: [(
                "codex:gpt-6-astra".to_owned(),
                "Include the API invariants in the handoff.".to_owned(),
            )]
            .into(),
            ..manifest
        };
        save(&root, &configured).unwrap();
        let body = fs::read_to_string(manifest_path(&root)).unwrap();
        assert!(body.ends_with("niles_schema: 2\n"), "{body}");
        assert_eq!(load(&root).unwrap(), Some(configured));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_yaml_formatted_manifest_still_round_trips() {
        let root = temp_test_path("manifest-serde-yaml-format");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(
            manifest_path(&root),
            r#"lead: claude:opus:medium
worker: codex:gpt-5.6-sol:medium
reviewer: claude:opus:medium
security: hermes:tencent/hy3:high
worker_planning:
  claude:haiku: |
    Settle the implementation approach and edge cases. Decompose the work into
    concrete changes and dispatch each change individually.
  codex:gpt-5.6-sol: |
    Supply the objective, constraints, and explicit acceptance criteria with
    minimal implementation granularity.
niles_schema: 2
"#,
        )
        .unwrap();
        let expected = WorkspaceManifest {
            lead: "claude:opus:medium".to_owned(),
            worker: "codex:gpt-5.6-sol:medium".to_owned(),
            reviewer: "claude:opus:medium".to_owned(),
            security: "hermes:tencent/hy3:high".to_owned(),
            worker_planning: [
                (
                    "claude:haiku".to_owned(),
                    "Settle the implementation approach and edge cases. Decompose the work into\nconcrete changes and dispatch each change individually.\n".to_owned(),
                ),
                (
                    "codex:gpt-5.6-sol".to_owned(),
                    "Supply the objective, constraints, and explicit acceptance criteria with\nminimal implementation granularity.\n".to_owned(),
                ),
            ]
            .into(),
            checkin: None,
            recheck: None,
        };

        let loaded = load(&root).unwrap().unwrap();

        assert_eq!(loaded, expected);
        save(&root, &loaded).unwrap();
        assert_eq!(load(&root).unwrap(), Some(expected));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_manifest_keeps_saphyr_line_and_column() {
        let root = temp_test_path("manifest-malformed-location");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(
            manifest_path(&root),
            "lead: claude\nworker_planning: [\nniles_schema: 2\n",
        )
        .unwrap();

        let err = load(&root).unwrap_err();
        let chain = err.chain().map(ToString::to_string).collect::<Vec<_>>();

        assert!(chain[0].contains("malformed YAML"), "{chain:?}");
        assert!(
            chain
                .iter()
                .any(|message| message.contains("line 2") && message.contains("column")),
            "{chain:?}"
        );
        assert!(
            chain
                .iter()
                .all(|message| !message.contains("worker_planning: [")),
            "{chain:?}"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_schema_stamps_are_rejected_before_deserialization() {
        for (label, stamp) in [("quoted", "\"2\""), ("null", "~")] {
            let root = temp_test_path(&format!("manifest-{label}-schema"));
            fs::create_dir_all(root.join(".niles")).unwrap();
            fs::write(
                manifest_path(&root),
                format!(
                    "lead: claude\nworker: codex\nreviewer: claude\nsecurity: claude\nniles_schema: {stamp}\n"
                ),
            )
            .unwrap();

            let err = load(&root).unwrap_err();

            assert!(
                err.to_string().contains("invalid niles_schema stamp"),
                "{label}: {err:#}"
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn worker_planning_loads_as_a_string_mapping() {
        let root = temp_test_path("manifest-worker-planning");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(
            manifest_path(&root),
            r#"
lead: claude
worker: codex:gpt-6-astra:high
reviewer: claude
security: claude
worker_planning:
  codex:gpt-6-astra: Include the API invariants in the handoff.
niles_schema: 2
"#,
        )
        .unwrap();

        let manifest = load(&root).unwrap().unwrap();

        assert_eq!(
            manifest.worker_planning.get("codex:gpt-6-astra"),
            Some(&"Include the API invariants in the handoff.".to_owned())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn worker_planning_rejects_non_string_values() {
        let root = temp_test_path("manifest-worker-planning-shape");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(
            manifest_path(&root),
            r#"
lead: claude
worker: codex
reviewer: claude
security: claude
worker_planning:
  codex:gpt-6-astra:
    steps: 2
niles_schema: 2
"#,
        )
        .unwrap();

        let err = format!("{:#}", load(&root).unwrap_err());

        assert!(err.contains("expected string"), "{err}");
        fs::remove_dir_all(root).unwrap();
    }
}
