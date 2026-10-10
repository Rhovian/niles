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
        let body = "lead: claude\nworker:\n  - when: Standard\n    models: [codex:gpt-6-sol, claude:opus]\n    efforts: [medium, high]\nreviewer: claude\nsecurity: claude\ndesign:\n  - models: [claude, codex]\n";
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
            let body = format!(
                "lead: claude\nworker:\n{worker}\nreviewer: claude\nsecurity: claude\ndesign:\n  - models: [claude, codex]\n"
            );
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
design:
  - models: [claude, codex]
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
design:
  - models: [claude, codex]
"#,
        )
        .unwrap();

        let manifest = load(&root).unwrap().unwrap();
        let bare = load(&anonymous).unwrap().unwrap();

        assert_eq!(manifest.checkin.as_deref(), Some("15m"));
        assert_eq!(manifest.recheck.as_deref(), Some("backoff"));
        assert_eq!(bare.checkin, None);
        assert_eq!(bare.recheck, None);

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
                "unknown field `manager`, expected one of lead, worker, reviewer, security, design, checkin, recheck"
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
            reviewer: "reviewbot".to_owned().into(),
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
            ..manifest
        };
        save(&root, &configured).unwrap();
        assert_eq!(load(&root).unwrap(), Some(configured));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn obsolete_and_missing_bindings_fail_with_the_manifest_path() {
        for (body, expected) in [
            (
                "lead: claude\nworker: codex\nreviewer: lead\nsecurity: claude\n",
                "reviewer: lead is no longer supported; reviewer must be an agent binding",
            ),
            (
                "lead: claude\nworker: codex\nreviewer: claude\nsecurity: claude\n",
                "missing field `design`",
            ),
            ("worker_planning: []\n", "unknown field `worker_planning`"),
        ] {
            let err = format!("{:#}", load_body("obsolete-bindings", body).unwrap_err());
            assert!(
                err.contains("manifest.yaml") && err.contains(expected),
                "{err}"
            );
        }
    }

    #[test]
    fn design_requires_two_distinct_families() {
        for design in [
            "claude:opus:high",
            "[{models: [claude:opus, claude:sonnet]}]",
            "[{models: [claude, CLAUDE]}]",
        ] {
            let body = format!(
                "lead: claude\nworker: codex\nreviewer: claude\nsecurity: claude\ndesign: {design}\n"
            );
            let err = format!("{:#}", load_body("design-families", &body).unwrap_err());
            assert!(
                err.contains("manifest.yaml")
                    && err.contains(
                        "design must list models from at least two agent families; found: claude"
                    ),
                "{err}"
            );
        }
    }

    #[test]
    fn malformed_manifest_keeps_saphyr_line_and_column() {
        let err =
            load_body("manifest-malformed-location", "lead: claude\nworker: [\n").unwrap_err();
        let chain = err.chain().map(ToString::to_string).collect::<Vec<_>>();

        assert!(chain[0].contains("manifest.yaml"), "{chain:?}");
        assert!(
            chain
                .iter()
                .any(|message| message.contains("line 2") && message.contains("column")),
            "{chain:?}"
        );
        assert!(
            chain.iter().all(|message| !message.contains("worker: [")),
            "{chain:?}"
        );
    }
}
