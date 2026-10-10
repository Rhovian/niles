use std::fs;

use super::*;
use crate::{agents::picker::Role, test_support::temp_test_path, workspace_manifest::save};

const MANIFEST: &str = "\
# workspace α
lead: claude
worker:
  - when: 'settled plan' # keep this spelling
    models: [codex:gpt-5.5]
    efforts: [medium, high]
  - when: design
    models: [claude:opus]
reviewer: lead
security: claude
worker_planning:
  - models: [codex:gpt-5.5]
    guidance: |
      Plan carefully.
      Then implement.
checkin: 15m
recheck: backoff
";

fn root(label: &str, body: &str) -> camino::Utf8PathBuf {
    let root = temp_test_path(label);
    fs::create_dir_all(root.join(".niles")).unwrap();
    fs::write(manifest_path(&root), body).unwrap();
    root
}

#[test]
fn changing_only_lead_preserves_worker_groups_and_planning() {
    let root = root("picker-preserve-groups", MANIFEST);
    let before = load(&root).unwrap().unwrap();
    let saved = ensure(&root, true, &mut Vec::new(), |_, mut draft, _| {
        draft.lead = "codex:gpt-6.1-sol:high".to_owned();
        Ok(Choice::Save(draft))
    })
    .unwrap()
    .unwrap();
    let after = load(&root).unwrap().unwrap();
    assert_eq!(after.lead, "codex:gpt-6.1-sol:high");
    assert_eq!(after.worker, before.worker);
    assert_eq!(after.worker_planning, before.worker_planning);
    assert_eq!(after.checkin, before.checkin);
    assert_eq!(after.recheck, before.recheck);
    assert_eq!(after, saved);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn continue_quit_and_unchanged_save_write_nothing() {
    let root = root("picker-no-write", MANIFEST);
    let path = manifest_path(&root);
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    let manifest = load(&root).unwrap();
    for (choice, launched) in [
        (Choice::Keep, &manifest),
        (Choice::Quit, &None),
        (Choice::Save(manifest.clone().unwrap()), &manifest),
    ] {
        let result = ensure(&root, true, &mut Vec::new(), |_, _, _| Ok(choice)).unwrap();
        assert_eq!(&result, launched);
        assert_eq!(fs::read_to_string(&path).unwrap(), MANIFEST);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn continuing_first_run_errors_without_a_file() {
    let root = temp_test_path("picker-first-continue");
    let error = ensure(&root, true, &mut Vec::new(), |_, _, _| Ok(Choice::Keep)).unwrap_err();
    assert_eq!(error.to_string(), "no workspace manifest written");
    assert!(!manifest_path(&root).exists());
}

#[test]
fn no_terminal_uses_existing_manifest_and_prints_roles() {
    let root = root("picker-noninteractive", MANIFEST);
    let mut output = Vec::new();
    let result = ensure(&root, false, &mut output, |_, _, _| {
        panic!("opened picker without terminal")
    })
    .unwrap();
    assert_eq!(result, load(&root).unwrap());
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("lead      claude")
    );
    assert_eq!(fs::read_to_string(manifest_path(&root)).unwrap(), MANIFEST);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn no_terminal_missing_or_malformed_manifest_errors() {
    let missing = temp_test_path("picker-missing");
    let error = ensure(&missing, false, &mut Vec::new(), |_, _, _| {
        panic!("opened picker")
    })
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("run `niles` from an interactive terminal")
    );
    let root = root("picker-malformed", "lead: [\n");
    let error = ensure(&root, false, &mut Vec::new(), |_, _, _| {
        panic!("opened picker")
    })
    .unwrap_err();
    assert_eq!(error.to_string(), load(&root).unwrap_err().to_string());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn every_preset_saves_and_loads_roles() {
    let config = crate::test_support::project_config("").unwrap();
    for (index, preset) in crate::agents::picker::load_presets(&config)
        .unwrap()
        .into_iter()
        .enumerate()
    {
        let root = temp_test_path(&format!("picker-preset-{index}"));
        let draft = ensure(&root, true, &mut Vec::new(), |_, mut draft, _| {
            for (role, value) in Role::ALL.into_iter().zip(preset.values.unwrap()) {
                role.set(&mut draft, value);
            }
            Ok(Choice::Save(draft))
        })
        .unwrap()
        .unwrap();
        let loaded = load(&root).unwrap().unwrap();
        assert_eq!(draft, loaded);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn changes_are_applied_to_latest_manifest() {
    let root = root("picker-reread", MANIFEST);
    let before = load(&root).unwrap().unwrap();
    let mut latest = before.clone();
    latest.checkin = Some("1h".to_owned());
    latest.security = "hermes".to_owned().into();
    save(&root, &latest).unwrap();
    let mut draft = before.clone();
    draft.lead = "codex".to_owned();
    let result = save_changes(&root, Some(&before), &draft).unwrap();
    assert_eq!(
        result,
        WorkspaceManifest {
            lead: "codex".to_owned(),
            ..latest
        }
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn only_a_single_model_and_effort_is_scalar() {
    let binding = |efforts: Option<&[&str]>| {
        crate::workspace_manifest::RoleBinding(vec![crate::workspace_manifest::AgentGroup {
            when: None,
            models: vec!["codex:gpt-5.5".to_owned()],
            efforts: efforts.map(|efforts| efforts.iter().map(|e| (*e).to_owned()).collect()),
        }])
    };
    assert_eq!(binding(None).scalar().as_deref(), Some("codex:gpt-5.5"));
    assert_eq!(
        binding(Some(&["high"])).scalar().as_deref(),
        Some("codex:gpt-5.5:high")
    );
    assert_eq!(binding(Some(&["medium", "high"])).scalar(), None);
}
