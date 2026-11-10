use super::WorkspaceManifest;
use super::setup::{ManifestFile, SetupSelection};
use crate::agents::ModelRoster;
use crate::config::spec;
use crate::skills::{self, files};
use crate::store;
#[cfg(test)]
use crate::store::atomic_write;
use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use std::fs;

pub fn manifest_path(root: &Utf8Path, selection: Option<&SetupSelection>) -> Result<Utf8PathBuf> {
    Ok(super::setup::active(root, selection)?.path().to_owned())
}

pub fn load(
    root: &Utf8Path,
    selection: Option<&SetupSelection>,
) -> Result<Option<WorkspaceManifest>> {
    let manifest = load_for_skill_listing(root, selection)?;
    if let Some(manifest) = &manifest {
        let path = manifest_path(root, selection)?;
        let global = skills::global_dir()?;
        skills::locate(&manifest.entry, root, &global)
            .with_context(|| format!("invalid entry skill in {path}"))?;
        for (skill, binding) in &manifest.bindings {
            skills::locate(skill, root, &global)
                .with_context(|| format!("invalid skill in {path}"))?;
            for name in binding.attached_skills() {
                skills::locate(name, root, &global)
                    .with_context(|| format!("invalid {skill} skill reference in {path}"))?;
            }
        }
    }
    Ok(manifest)
}

/// Defer skill checks so discovery can warn per skill without losing project settings.
pub(crate) fn load_for_skill_listing(
    root: &Utf8Path,
    selection: Option<&SetupSelection>,
) -> Result<Option<WorkspaceManifest>> {
    let file = super::setup::active(root, selection)?;
    let path = file.path();
    let mut manifest: Option<WorkspaceManifest> = match &file {
        ManifestFile::Default(_) => store::read_optional_yaml(path)?,
        ManifestFile::Named(source) => Some(
            store::parse_yaml(
                &files::read(path, source.setup.origin)
                    .with_context(|| format!("failed to read setup {source}"))?,
            )
            .with_context(|| format!("failed to parse setup {source}"))?,
        ),
    };
    if let Some(manifest) = &mut manifest {
        if let ManifestFile::Named(source) = &file {
            manifest.source = Some(source.clone());
        }
        let config = spec::load_project_config_from(root)?;
        manifest
            .tiers
            .dedup(&config.models)
            .with_context(|| format!("invalid manifest {path}"))?;
        check(manifest, &config.models).with_context(|| format!("invalid manifest {path}"))?;
    }
    Ok(manifest)
}

pub(crate) fn check(manifest: &WorkspaceManifest, models: &ModelRoster) -> Result<()> {
    for agent in manifest.tiers.agents() {
        super::types::exact_spec(agent, models).context("invalid manifest agent")?;
    }
    manifest
        .tiers
        .admit(&manifest.entry_agent, models)
        .context("invalid entry_agent")
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
    use crate::store::{self, paths};
    use crate::test_support::default_manifest::{load, manifest_path, save};
    use crate::test_support::{self, manifest_body, skill, temp_dir};

    const TIERS: &str = "tiers: {standard: {primary: claude:sonnet:high, secondaries: []}}\n";

    fn load_body(label: &str, body: &str) -> Result<Option<WorkspaceManifest>> {
        let root = temp_dir(label);
        fs::create_dir_all(paths::project_dir(&root)?)?;
        atomic_write(manifest_path(&root).unwrap(), body)?;
        let result = load(&root);
        test_support::remove_workspace(&root)?;
        result
    }

    #[test]
    fn invalid_manifests_report_the_path_and_cause() {
        let body = manifest_body();
        let mut cases = vec![
            ("no-entry", TIERS.to_owned(), "missing field `entry`"),
            ("absent-entry", format!("entry: absent\nentry_agent: claude:sonnet:high\n{TIERS}"), "skill `absent` has no SKILL.md"),
            ("missing-tiers", "entry: lead\nentry_agent: claude:sonnet:high\n".to_owned(), "missing field `tiers`"),
            ("unknown-tier", body.replacen("standard:", "unknown:", 1), "unknown field `unknown`"),
            ("no-effort", body.replacen("claude:sonnet:high", "claude:sonnet", 1), "must name an exact effort"),
            ("bad-effort", body.replacen("claude:sonnet:high", "claude:sonnet:invalid", 1), "invalid"),
            ("no-model", body.replacen("claude:sonnet:high", "claude", 1), "must name an exact"),
            ("legacy", "lead: claude\nworker: codex\n".to_owned(), "missing field `entry`"),
            ("missing-standard", body.replacen("  standard:\n    primary: codex:gpt-6.1-sol:medium\n    secondaries: [claude:sonnet:high]\n", "", 1), "missing field `standard`"),
            ("unknown-field", body.replacen("primary:", "when:", 1), "unknown field `when`"),
            ("missing-secondaries", body.replacen("    secondaries: [claude:sonnet:high]\n", "", 1), "missing field `secondaries`"),
            ("missing-entry-agent", body.replacen("entry_agent: claude:opus:high\n", "", 1), "missing field `entry_agent`"),
            ("unlisted-entry-agent", body.replacen("entry_agent: claude:opus:high", "entry_agent: claude:opus:medium", 1), "agent claude:opus:medium is not listed in the manifest tiers"),
        ];
        for (name, cause) in [
            ("entry", "expected string scalar"),
            ("checkin", "expected string scalar"),
            ("recheck", "expected string scalar"),
            ("worktree", "unknown field `standard`"),
        ] {
            cases.push((
                name,
                format!(
                    "{name}: {{standard: {{primary: claude:sonnet:high, secondaries: []}}}}\n{body}"
                ),
                cause,
            ));
        }
        for fields in [
            "standard: {primary: claude:sonnet:high, secondaries: []}",
            "default_tier: economy",
            "tiers: {}",
        ] {
            cases.push((
                "skill-model",
                format!("{body}custom: {{{fields}}}\n"),
                "unknown field",
            ));
        }
        for (label, body, cause) in cases {
            let error = format!("{:#}", load_body(label, &body).unwrap_err());
            assert!(
                error.contains("manifest.yaml") && error.contains(cause),
                "{label}: {error}"
            );
        }
    }

    #[test]
    fn entry_needs_no_composition_binding() {
        let body = format!("entry: reviewer\nentry_agent: claude:sonnet:high\n{TIERS}");
        let manifest = load_body("manifest-entry", &body).unwrap().unwrap();
        assert_eq!(manifest.entry.as_str(), "reviewer");
        assert!(manifest.bindings.is_empty());
    }

    #[test]
    fn exact_agents_and_cadence_round_trip() {
        let root = temp_dir("manifest-invalid-tiers");
        fs::create_dir_all(paths::project_dir(&root).unwrap()).unwrap();
        let mut manifest = test_support::manifest();
        manifest.tiers.standard.secondaries =
            vec!["claude:haiku".into(), "claude:sonnet:high".into()];
        manifest.checkin = Some("15m".into());
        manifest.recheck = Some("backoff".into());
        save(&root, &manifest).unwrap();
        assert_eq!(load(&root).unwrap(), Some(manifest));
        test_support::remove_workspace(&root).unwrap();
    }

    #[test]
    fn malformed_manifest_keeps_saphyr_line_and_column() {
        let err = load_body(
            "manifest-malformed-location",
            "lead: {standard: {primary: claude:sonnet:high, secondaries: []}}\nworker: [\n",
        )
        .unwrap_err();
        let chain = err.chain().map(ToString::to_string).collect::<Vec<_>>();

        assert!(
            chain.first().unwrap().contains("manifest.yaml"),
            "{chain:?}"
        );
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
        let root = temp_dir("manifest-save-cadence");
        let manifest = test_support::manifest();
        let path = manifest_path(&root).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        atomic_write(&path, test_support::manifest_body()).unwrap();
        save(&root, &manifest).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(load(&root).unwrap(), Some(manifest.clone()));
        assert!(body.lines().any(|line| line == "entry: lead"));
        assert!(
            body.lines()
                .filter(|line| !line.starts_with(' '))
                .map(|line| line.split_once(':').unwrap().0)
                .filter(|key| !["entry", "entry_agent", "tiers"].contains(key))
                .is_sorted()
        );
        for gone in ["checkin:", "recheck:", "worktree:"] {
            assert!(!body.contains(gone), "{body}");
        }
        let mut configured = WorkspaceManifest {
            checkin: Some("15m".into()),
            recheck: Some("backoff".into()),
            worktree: Some(
                store::parse_yaml("env: {CACHE: /tmp/cache}\nsetup: echo ready").unwrap(),
            ),
            ..manifest
        };
        configured.bindings.get_mut(&skill("worker")).unwrap().gates =
            Some(store::parse_yaml("post: [{run: [true], timeout: 1500ms}]").unwrap());
        save(&root, &configured).unwrap();
        assert_eq!(load(&root).unwrap(), Some(configured));
        test_support::remove_workspace(&root).unwrap();
    }
}
