use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{store, store::paths::NILES_DIR};

use super::WorkspaceManifest;

pub fn manifest_path(root: &Utf8Path) -> Utf8PathBuf {
    root.join(NILES_DIR).join("manifest.yaml")
}

pub fn load(root: &Utf8Path) -> Result<Option<WorkspaceManifest>> {
    let path = manifest_path(root);
    store::read_optional_yaml(&path)
}

pub fn save(root: &Utf8Path, manifest: &WorkspaceManifest) -> Result<()> {
    let path = manifest_path(root);
    let parent = root.join(NILES_DIR);
    fs::create_dir_all(&parent).with_context(|| format!("failed to create {parent}"))?;
    store::write_yaml(&path, manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::temp_test_path;

    fn load_body(label: &str, body: &str) -> Result<Option<WorkspaceManifest>> {
        let root = temp_test_path(label);
        fs::create_dir_all(root.join(NILES_DIR))?;
        fs::write(manifest_path(&root), body)?;
        let result = load(&root);
        fs::remove_dir_all(root)?;
        result
    }

    #[test]
    fn groups_load_and_scalar_saves_as_scalar() {
        let body = "lead: claude\nworker:\n  - when: Standard\n    models: [codex:gpt-6-sol, claude:opus]\n    efforts: [medium, high]\nreviewer: lead\nsecurity: claude\n";
        let manifest = load_body("manifest-groups", body).unwrap().unwrap();
        assert_eq!(
            manifest.worker.agents().collect::<Vec<_>>(),
            ["codex:gpt-6-sol", "claude:opus"]
        );
        let root = temp_test_path("manifest-scalar-roundtrip");
        save(&root, &WorkspaceManifest::default()).unwrap();
        let saved = fs::read_to_string(manifest_path(&root)).unwrap();
        assert!(saved.contains("worker: codex"), "{saved}");
        fs::remove_dir_all(root).unwrap();
        for (name, worker) in [
            ("empty", "  - when: Standard\n    models: []"),
            (
                "unknown",
                "  - when: Standard\n    models: [codex]\n    extra: no",
            ),
            (
                "empty-efforts",
                "  - when: Standard\n    models: [codex]\n    efforts: []",
            ),
        ] {
            let body =
                format!("lead: claude\nworker:\n{worker}\nreviewer: lead\nsecurity: claude\n");
            assert!(load_body(name, &body).is_err());
        }
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
        let err = format!(
            "{:#}",
            load_body(
                "manifest-pre-lead",
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
"#,
            )
            .unwrap_err()
        );

        // Names the offending field and the fields that replaced it.
        assert!(err.contains("unknown field `manager`"), "{err}");
        assert!(
            err.contains(
                "unknown field `manager`, expected one of lead, worker, reviewer, security, worker_planning, checkin, recheck"
            ),
            "{err}"
        );
    }

    #[test]
    fn saving_manifest_writes_role_bindings_and_nothing_else() {
        let root = temp_test_path("manifest-save");
        let manifest = WorkspaceManifest {
            lead: "claude".to_owned(),
            worker: "codebot".to_owned().into(),
            reviewer: super::super::ReviewerBinding::Agent("reviewbot".to_owned().into()),
            security: "auditbot".to_owned().into(),
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
        assert_eq!(load(&root).unwrap(), Some(configured));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lead_reviewer_round_trips_as_scalar() {
        let root = temp_test_path("manifest-lead-reviewer");
        let manifest = WorkspaceManifest {
            reviewer: super::super::ReviewerBinding::Lead,
            ..WorkspaceManifest::default()
        };
        save(&root, &manifest).unwrap();
        let body = fs::read_to_string(manifest_path(&root)).unwrap();
        assert!(body.contains("reviewer: lead\n"), "{body}");
        assert_eq!(load(&root).unwrap(), Some(manifest));
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
"#,
        )
        .unwrap();
        let expected = WorkspaceManifest {
            lead: "claude:opus:medium".to_owned(),
            worker: "codex:gpt-5.6-sol:medium".to_owned().into(),
            reviewer: super::super::ReviewerBinding::Agent("claude:opus:medium".to_owned().into()),
            security: "hermes:tencent/hy3:high".to_owned().into(),
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
        let err = load_body(
            "manifest-malformed-location",
            "lead: claude\nworker_planning: [\n",
        )
        .unwrap_err();
        let chain = err.chain().map(ToString::to_string).collect::<Vec<_>>();

        assert!(chain[0].contains("manifest.yaml"), "{chain:?}");
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
    }

    #[test]
    fn worker_planning_rejects_non_string_values() {
        let err = format!(
            "{:#}",
            load_body(
                "manifest-worker-planning-shape",
                r#"
lead: claude
worker: codex
reviewer: claude
security: claude
worker_planning:
  codex:gpt-6-astra:
    steps: 2
"#,
            )
            .unwrap_err()
        );

        assert!(err.contains("expected string"), "{err}");
    }
}
