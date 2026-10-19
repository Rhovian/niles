use std::fs;

use super::*;
use crate::{test_support::temp_test_path, workspace_manifest::save};

const MANIFEST: &str = include_str!("../../../examples/claude-lead.yaml");

fn root(label: &str, body: &str) -> camino::Utf8PathBuf {
    let root = temp_test_path(label);
    fs::create_dir_all(crate::store::paths::project_dir(&root).unwrap()).unwrap();
    fs::write(manifest_path(&root).unwrap(), body).unwrap();
    root
}

#[test]
fn changing_only_lead_preserves_worker_and_design_tiers() {
    let root = root("picker-preserve-tiers", MANIFEST);
    let before = load(&root).unwrap().unwrap();
    let saved = ensure(&root, true, &mut Vec::new(), |_, mut draft, _, _| {
        draft.lead = "codex:gpt-6.1-sol:high".to_owned().into();
        Ok(Choice::Save(Box::new(draft)))
    })
    .unwrap()
    .unwrap();
    let after = load(&root).unwrap().unwrap();
    assert_eq!(after.lead.standard.primary, "codex:gpt-6.1-sol:high");
    assert_eq!(after.worker, before.worker);
    assert_eq!(after.design, before.design);
    assert_eq!(after.checkin, before.checkin);
    assert_eq!(after.recheck, before.recheck);
    assert_eq!(after, saved);
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn continue_quit_and_unchanged_save_write_nothing() {
    let root = root("picker-no-write", MANIFEST);
    let path = manifest_path(&root).unwrap();
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    let manifest = load(&root).unwrap();
    for (choice, launched) in [
        (Choice::Keep, &manifest),
        (Choice::Quit, &None),
        (Choice::Save(Box::new(manifest.clone().unwrap())), &manifest),
    ] {
        let result = ensure(&root, true, &mut Vec::new(), |_, _, _, _| Ok(choice)).unwrap();
        assert_eq!(&result, launched);
        assert_eq!(fs::read_to_string(&path).unwrap(), MANIFEST);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    }
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn first_run_offers_saving_unchanged_defaults_and_writes_them() {
    let root = temp_test_path("picker-first-save");
    let result = ensure(&root, true, &mut Vec::new(), |_, draft, _, reason| {
        assert!(reason.is_some(), "missing manifest must require saving");
        assert_eq!(draft, WorkspaceManifest::default());
        Ok(Choice::Save(Box::new(draft)))
    })
    .unwrap();
    assert_eq!(result, Some(WorkspaceManifest::default()));
    assert_eq!(load(&root).unwrap(), result);
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn no_terminal_uses_existing_manifest_and_prints_roles() {
    let root = root("picker-noninteractive", MANIFEST);
    let mut output = Vec::new();
    let result = ensure(&root, false, &mut output, |_, _, _, _| {
        panic!("opened picker without terminal")
    })
    .unwrap();
    assert_eq!(result, load(&root).unwrap());
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("lead      claude")
    );
    assert_eq!(
        fs::read_to_string(manifest_path(&root).unwrap()).unwrap(),
        MANIFEST
    );
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn no_terminal_missing_or_malformed_manifest_errors() {
    let missing = temp_test_path("picker-missing");
    let error = ensure(&missing, false, &mut Vec::new(), |_, _, _, _| {
        panic!("opened picker")
    })
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("run `niles` from an interactive terminal")
    );
    let root = root("picker-malformed", "lead: [\n");
    let error = ensure(&root, false, &mut Vec::new(), |_, _, _, _| {
        panic!("opened picker")
    })
    .unwrap_err();
    assert_eq!(error.to_string(), load(&root).unwrap_err().to_string());
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn changes_are_applied_to_latest_manifest() {
    let root = root("picker-reread", MANIFEST);
    let before = load(&root).unwrap().unwrap();
    let mut latest = before.clone();
    latest.checkin = Some("1h".to_owned());
    latest.security.standard.primary = "hermes:z-ai/glm-5.3-flash:high".into();
    save(&root, &latest).unwrap();
    let mut draft = before.clone();
    draft.lead.standard.primary = "codex:gpt-6.1-sol:medium".into();
    draft.design.standard.secondaries.reverse();
    draft.research.standard.primary = "claude:sonnet:high".into();
    let result = save_changes(&root, Some(&before), &draft).unwrap();
    assert_eq!(
        result,
        WorkspaceManifest {
            lead: draft.lead,
            design: draft.design,
            research: draft.research,
            ..latest
        }
    );
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn invalid_manifests_reopen_with_defaults_and_save_a_complete_replacement() {
    for body in [
        "lead: claude\nworker: codex\nreviewer: claude\nsecurity: claude\n",
        "manager: claude\n",
        "lead: [\n",
    ] {
        let root = root("picker-replace", body);
        let saved = ensure(&root, true, &mut Vec::new(), |_, draft, _, reason| {
            assert_eq!(draft, WorkspaceManifest::default());
            let reason = reason.unwrap();
            assert!(reason.contains("Cannot parse manifest"), "{reason}");
            assert!(reason.contains("manifest.yaml"), "{reason}");
            Ok(Choice::Save(Box::new(draft)))
        })
        .unwrap()
        .unwrap();
        assert_eq!(saved, WorkspaceManifest::default());
        assert_eq!(load(&root).unwrap(), Some(saved));
        crate::test_support::remove_workspace(&root).unwrap();
    }
}

#[test]
fn quitting_replacement_preserves_the_invalid_manifest() {
    let body = "lead: claude\n";
    let root = root("picker-replace-quit", body);
    let result = ensure(&root, true, &mut Vec::new(), |_, _, _, reason| {
        assert!(reason.is_some());
        Ok(Choice::Quit)
    })
    .unwrap();
    assert_eq!(result, None);
    assert_eq!(
        fs::read_to_string(manifest_path(&root).unwrap()).unwrap(),
        body
    );
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn manifest_read_failures_do_not_open_replacement() {
    let root = temp_test_path("picker-read-failure");
    fs::create_dir_all(manifest_path(&root).unwrap()).unwrap();
    let error = ensure(&root, true, &mut Vec::new(), |_, _, _, _| {
        panic!("opened picker for a read failure")
    })
    .unwrap_err();
    assert!(error.to_string().contains("failed to read"), "{error:#}");
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn noninteractive_stale_manifest_fails_without_writing() {
    let body = "lead: claude\nworker: codex\nreviewer: claude\nsecurity: claude\n";
    let root = root("picker-stale-noninteractive", body);
    let error = ensure(&root, false, &mut Vec::new(), |_, _, _, _| {
        panic!("opened picker without terminal")
    })
    .unwrap_err();
    assert!(format!("{error:#}").contains("manifest.yaml"));
    assert_eq!(
        fs::read_to_string(manifest_path(&root).unwrap()).unwrap(),
        body
    );
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn replacement_write_failure_stops_launch() {
    let root = root("picker-replace-write-failure", "lead: [\n");
    let error = ensure(&root, true, &mut Vec::new(), |_, draft, _, _| {
        fs::remove_file(manifest_path(&root).unwrap()).unwrap();
        fs::create_dir(manifest_path(&root).unwrap()).unwrap();
        Ok(Choice::Save(Box::new(draft)))
    })
    .unwrap_err();
    assert!(error.to_string().contains("failed to write"), "{error:#}");
    crate::test_support::remove_workspace(&root).unwrap();
}
