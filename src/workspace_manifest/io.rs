use std::fs;

use anyhow::{Context, Result, ensure};
use camino::{Utf8Path, Utf8PathBuf};

use crate::store;

use super::{
    WorkspaceManifest,
    setup::{ManifestFile, SetupSelection},
};

pub fn manifest_path(root: &Utf8Path, selection: Option<&SetupSelection>) -> Result<Utf8PathBuf> {
    Ok(super::setup::active(root, selection)?.path().to_owned())
}

pub fn load(
    root: &Utf8Path,
    selection: Option<&SetupSelection>,
) -> Result<Option<WorkspaceManifest>> {
    let file = super::setup::active(root, selection)?;
    let path = file.path();
    let mut manifest: Option<WorkspaceManifest> = match &file {
        ManifestFile::Default(_) => store::read_optional_yaml(path)?,
        ManifestFile::Named(source) => Some(
            store::parse_yaml(
                &crate::skills::files::read(path, source.setup.origin)
                    .with_context(|| format!("failed to read setup {source}"))?,
            )
            .with_context(|| format!("failed to parse setup {source}"))?,
        ),
    };
    if let Some(manifest) = &mut manifest {
        if let ManifestFile::Named(source) = &file {
            manifest.source = Some(source.clone());
        }
        let config = crate::config::spec::load_project_config_from(root)?;
        manifest
            .binding(&manifest.entry)
            .with_context(|| format!("invalid entry in {path}"))?;
        let global = crate::skills::global_dir()?;
        for (skill, binding) in &manifest.bindings {
            crate::skills::locate(skill, root, &global)
                .with_context(|| format!("invalid skill in {path}"))?;
            for name in binding
                .skills
                .iter()
                .chain(&binding.always)
                .chain(&binding.on_from)
                .chain(&binding.on_stage)
            {
                let source = crate::skills::locate(name, root, &global)
                    .with_context(|| format!("invalid {skill} skill reference in {path}"))?;
                ensure!(
                    !binding.skills.contains(name)
                        || !matches!(source, crate::skills::Source::Builtin(_)),
                    "menu skill {name} ({source}) in {path} needs a disk SKILL.md"
                );
            }
            for agent in binding.agents() {
                super::types::exact_spec(agent, &config.models)
                    .with_context(|| format!("invalid {skill} agent in {path}"))?;
            }
        }
    }
    Ok(manifest)
}

pub fn save(
    root: &Utf8Path,
    manifest: &WorkspaceManifest,
    selection: Option<&SetupSelection>,
) -> Result<()> {
    let file = super::setup::active(root, selection)?;
    let path = file.path();
    let parent = path.parent().context("manifest has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    store::write_yaml(path, manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::default_manifest::{load, manifest_path, save};
    use crate::test_support::skill;
    use crate::test_support::temp_test_path;

    fn load_body(label: &str, body: &str) -> Result<Option<WorkspaceManifest>> {
        let root = temp_test_path(label);
        fs::create_dir_all(crate::store::paths::project_dir(&root)?)?;
        fs::write(manifest_path(&root).unwrap(), body)?;
        let result = load(&root);
        crate::test_support::remove_workspace(&root)?;
        result
    }

    #[test]
    fn only_entry_must_be_bound() {
        let body = "entry: reviewer\nreviewer: {standard: {primary: claude:sonnet:high, secondaries: []}}\n";
        let manifest = load_body("manifest-entry", body).unwrap().unwrap();
        assert_eq!(manifest.entry.as_str(), "reviewer");
        assert_eq!(manifest.bindings.len(), 1);
        let error = format!(
            "{:#}",
            load_body(
                "manifest-entry",
                &body.replace("entry: reviewer", "entry: missing")
            )
            .unwrap_err()
        );
        assert!(
            error.contains("missing") && error.contains("manifest.yaml"),
            "{error}"
        );
    }

    #[test]
    fn reserved_settings_cannot_be_binding_keys() {
        for (name, cause) in [
            ("entry", "expected string scalar"),
            ("checkin", "expected string scalar"),
            ("recheck", "expected string scalar"),
            ("worktree", "unknown field `standard`"),
        ] {
            let body = format!(
                "{name}: {{standard: {{primary: claude:sonnet:high, secondaries: []}}}}\n{}",
                crate::test_support::manifest_body()
            );
            let error = format!("{:#}", load_body("manifest-reserved", &body).unwrap_err());
            assert!(
                error.contains("manifest.yaml") && error.contains(cause),
                "{name}: {error}"
            );
        }
    }

    #[test]
    fn shipped_templates_load_and_resolve_every_exact_pair() {
        let skill = include_str!("../../skills/niles-setup/SKILL.md");
        let templates = skill
            .split("```yaml\n")
            .skip(1)
            .map(|block| block.split_once("```").unwrap().0);
        let root = temp_test_path("manifest-templates");
        for body in templates {
            fs::create_dir_all(crate::store::paths::project_dir(&root).unwrap()).unwrap();
            fs::write(manifest_path(&root).unwrap(), body).unwrap();
            let manifest = load(&root).unwrap().unwrap();
            save(&root, &manifest).unwrap();
            assert_eq!(load(&root).unwrap(), Some(manifest));
        }
        crate::test_support::remove_workspace(&root).unwrap();
    }

    #[test]
    fn setup_skills_ship_only_their_configured_tiers() {
        use crate::agents::roster::Tier;
        let manifest = crate::test_support::manifest();
        for (skill, frontier, economy, mechanical) in [
            ("lead", true, false, false),
            ("worker", true, true, true),
            ("design", true, false, false),
            ("reviewer", true, true, false),
            ("security", false, true, true),
            ("research", false, true, false),
        ] {
            let binding = manifest.binding(&skill.parse().unwrap()).unwrap();
            assert!(binding.tier(Tier::Standard).is_ok());
            assert_eq!(binding.tier(Tier::Frontier).is_ok(), frontier);
            assert_eq!(binding.tier(Tier::Economy).is_ok(), economy);
            assert_eq!(binding.tier(Tier::Mechanical).is_ok(), mechanical);
        }
    }

    #[test]
    fn invalid_schema_and_inexact_agents_error_with_path() {
        let root = temp_test_path("manifest-invalid-tiers");
        fs::create_dir_all(crate::store::paths::project_dir(&root).unwrap()).unwrap();
        let body = crate::test_support::manifest_body();
        for invalid in [
            body.replacen("standard:", "unknown:", 1),
            body.replacen("claude:sonnet:high", "claude:sonnet", 1),
            body.replacen("claude:sonnet:high", "claude:sonnet:invalid", 1),
            body.replacen("claude:sonnet:high", "claude", 1),
            "lead: claude\nworker: codex\n".to_owned(),
        ] {
            fs::write(manifest_path(&root).unwrap(), invalid).unwrap();
            let error = format!("{:#}", load(&root).unwrap_err());
            assert!(error.contains("manifest.yaml"), "{error}");
        }
        let mut manifest = crate::test_support::manifest();
        manifest
            .bindings
            .get_mut(&skill("worker"))
            .unwrap()
            .standard
            .secondaries = vec!["claude:haiku".into(), "claude:sonnet:high".into()];
        manifest.checkin = Some("15m".into());
        manifest.recheck = Some("backoff".into());
        save(&root, &manifest).unwrap();
        assert_eq!(load(&root).unwrap(), Some(manifest));
        crate::test_support::remove_workspace(&root).unwrap();
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
    fn save_sorts_bindings_preserves_cadence_and_omits_absent_keys() {
        let root = temp_test_path("manifest-save-cadence");
        let manifest = crate::test_support::manifest();
        let path = manifest_path(&root).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, crate::test_support::manifest_body()).unwrap();
        save(&root, &manifest).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(load(&root).unwrap(), Some(manifest.clone()));
        assert!(body.lines().any(|line| line == "entry: lead"));
        assert!(
            body.lines()
                .filter(|line| !line.starts_with(' '))
                .map(|line| line.split_once(':').unwrap().0)
                .filter(|key| *key != "entry")
                .is_sorted()
        );
        for gone in ["checkin:", "recheck:", "worktree:"] {
            assert!(!body.contains(gone), "{body}");
        }
        let mut configured = WorkspaceManifest {
            checkin: Some("15m".into()),
            recheck: Some("backoff".into()),
            worktree: Some(
                crate::store::parse_yaml("env: {CACHE: /tmp/cache}\nsetup: echo ready").unwrap(),
            ),
            ..manifest
        };
        configured.bindings.get_mut(&skill("worker")).unwrap().gates =
            Some(crate::store::parse_yaml("post: [{run: [true], timeout: 1500ms}]").unwrap());
        save(&root, &configured).unwrap();
        assert_eq!(load(&root).unwrap(), Some(configured));
        crate::test_support::remove_workspace(&root).unwrap();
    }

    #[test]
    fn required_skill_and_tier_fields_fail_with_manifest_path() {
        let body = crate::test_support::manifest_body();
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
