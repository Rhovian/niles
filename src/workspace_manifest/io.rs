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
    let manifest: Option<WorkspaceManifest> = store::read_optional_yaml(&path)?;
    if let Some(manifest) = &manifest {
        let config = crate::config::spec::load_project_config_from(root)?;
        for role in crate::agents::picker::Role::ALL {
            for agent in role.binding(manifest).agents() {
                super::types::exact_spec(agent, &config.models)
                    .with_context(|| format!("invalid {} agent in {path}", role.name()))?;
            }
        }
    }
    Ok(manifest)
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
    use crate::{agents::picker::Role, test_support::temp_test_path};

    fn load_body(label: &str, body: &str) -> Result<Option<WorkspaceManifest>> {
        let root = temp_test_path(label);
        fs::create_dir_all(root.join(".niles"))?;
        fs::write(manifest_path(&root), body)?;
        let result = load(&root);
        fs::remove_dir_all(root)?;
        result
    }

    #[test]
    fn shipped_templates_load_and_resolve_every_exact_pair() {
        let skill = include_str!("../../skills/niles-setup/SKILL.md");
        let templates = skill
            .split("```yaml\n")
            .skip(1)
            .map(|block| block.split_once("```").unwrap().0)
            .chain([
                include_str!("../../examples/claude-lead.yaml"),
                include_str!("../../examples/codex-lead.yaml"),
            ]);
        let root = temp_test_path("manifest-templates");
        for body in templates {
            fs::create_dir_all(root.join(".niles")).unwrap();
            fs::write(manifest_path(&root), body).unwrap();
            let manifest = load(&root).unwrap().unwrap();
            save(&root, &manifest).unwrap();
            assert_eq!(load(&root).unwrap(), Some(manifest));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn default_roles_ship_only_their_configured_tiers() {
        use crate::agents::roster::Tier;
        let manifest = WorkspaceManifest::default();
        for (role, frontier, economy, mechanical) in [
            (Role::Lead, true, false, false),
            (Role::Worker, true, true, true),
            (Role::Design, true, false, false),
            (Role::Reviewer, true, true, false),
            (Role::Security, false, true, true),
            (Role::Research, false, true, false),
        ] {
            let binding = role.binding(&manifest);
            assert!(binding.tier(Tier::Standard).is_ok());
            assert_eq!(binding.tier(Tier::Frontier).is_ok(), frontier);
            assert_eq!(binding.tier(Tier::Economy).is_ok(), economy);
            assert_eq!(binding.tier(Tier::Mechanical).is_ok(), mechanical);
        }
    }

    #[test]
    fn invalid_schema_and_inexact_agents_error_with_path() {
        let root = temp_test_path("manifest-invalid-tiers");
        fs::create_dir_all(root.join(".niles")).unwrap();
        let body = include_str!("../../examples/claude-lead.yaml");
        for invalid in [
            body.replacen("standard:", "unknown:", 1),
            body.replacen("primary:", "when:", 1),
            body.replacen("claude:sonnet:high", "claude:sonnet", 1),
            body.replacen("claude:sonnet:high", "claude:sonnet:invalid", 1),
            body.replacen("claude:sonnet:high", "claude", 1),
            "lead: claude\nworker: codex\n".to_owned(),
        ] {
            fs::write(manifest_path(&root), invalid).unwrap();
            let error = format!("{:#}", load(&root).unwrap_err());
            assert!(error.contains("manifest.yaml"), "{error}");
        }
        let mut manifest = WorkspaceManifest::default();
        manifest.worker.standard.secondaries =
            vec!["claude:haiku".into(), "claude:sonnet:high".into()];
        manifest.checkin = Some("15m".into());
        manifest.recheck = Some("backoff".into());
        save(&root, &manifest).unwrap();
        assert_eq!(load(&root).unwrap(), Some(manifest));
        fs::remove_dir_all(root).unwrap();
    }
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
                "unknown field `manager`, expected one of lead, worker, reviewer, security, design, research, checkin, recheck"
            ),
            "{err}"
        );
    }
    #[test]
    fn malformed_manifest_keeps_saphyr_line_and_column() {
        let err = load_body(
            "manifest-malformed-location",
            "lead: {standard: {primary: claude:sonnet:high, secondaries: []}}\nworker: [\n",
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
            chain.iter().all(|message| !message.contains("worker: [")),
            "{chain:?}"
        );
    }
    #[test]
    fn saving_manifest_preserves_cadence_and_omits_absent_keys() {
        let root = temp_test_path("manifest-save-cadence");
        let manifest = WorkspaceManifest::default();
        save(&root, &manifest).unwrap();
        let body = fs::read_to_string(manifest_path(&root)).unwrap();
        assert_eq!(load(&root).unwrap(), Some(manifest.clone()));
        for gone in [
            "manager:",
            "planner:",
            "validation_command:",
            "flow:",
            "checkin:",
            "recheck:",
            "worker_planning:",
            "when:",
        ] {
            assert!(!body.contains(gone), "{body}");
        }
        let configured = WorkspaceManifest {
            checkin: Some("15m".into()),
            recheck: Some("backoff".into()),
            ..manifest
        };
        save(&root, &configured).unwrap();
        assert_eq!(load(&root).unwrap(), Some(configured));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn required_role_and_tier_fields_fail_with_manifest_path() {
        let body = include_str!("../../examples/claude-lead.yaml");
        for (invalid, cause) in [
            (
                body.replacen("standard:", "economy:", 1),
                "missing field `standard`",
            ),
            (
                body.replacen("primary:", "when:", 1),
                "unknown field `when`",
            ),
            (
                body.replacen("    secondaries: [codex:gpt-6.1-sol:medium]\n", "", 1),
                "missing field `secondaries`",
            ),
            (
                body.split_once("research:").unwrap().0.to_owned(),
                "missing field `research`",
            ),
        ] {
            let error = format!(
                "{:#}",
                load_body("manifest-required-fields", &invalid).unwrap_err()
            );
            assert!(
                error.contains("manifest.yaml") && error.contains(cause),
                "{error}"
            );
        }
    }
}
