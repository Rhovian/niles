use crate::agents::roster::Tier;
use std::fs;

use super::*;
fn never(_: &Utf8Path) -> Result<Option<WorkspaceManifest>> {
    panic!("setup with an existing manifest or no terminal")
}
use crate::test_support::{
    default_manifest::{load, manifest_path},
    manifest_body, remove_workspace, temp_test_path,
};

fn root(label: &str, body: &str) -> camino::Utf8PathBuf {
    let root = temp_test_path(label);
    fs::create_dir_all(crate::store::paths::project_dir(&root).unwrap()).unwrap();
    fs::write(manifest_path(&root).unwrap(), body).unwrap();
    root
}

#[test]
fn changing_standard_preserves_composition_and_other_tiers() {
    let root = root("picker-preserve-tiers", manifest_body());
    let before = load(&root).unwrap().unwrap();
    let mut output = vec![];
    let saved = ensure(&root, None, true, &mut output, never, |_, mut draft, _| {
        draft.tiers.standard.primary = "codex:gpt-6.1-sol:high".into();
        Ok(Choice::Save(Box::new(draft)))
    })
    .unwrap()
    .unwrap();
    let after = load(&root).unwrap().unwrap();
    let mut expected = before;
    let standard = &mut expected.tiers.standard;
    standard.primary = "codex:gpt-6.1-sol:high".into();
    assert_eq!(after, expected);
    assert_eq!(after, saved);
    remove_workspace(&root).unwrap();
}

#[test]
fn continue_quit_and_unchanged_save_write_nothing() {
    let root = root("picker-no-write", manifest_body());
    let path = manifest_path(&root).unwrap();
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    let manifest = load(&root).unwrap();
    for (choice, launched) in [
        (Choice::Keep, &manifest),
        (Choice::Quit, &None),
        (Choice::Save(Box::new(manifest.clone().unwrap())), &manifest),
    ] {
        let result = ensure(&root, None, true, &mut vec![], never, |_, _, _| Ok(choice)).unwrap();
        assert_eq!(&result, launched);
        assert_eq!(fs::read_to_string(&path).unwrap(), manifest_body());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    }
    remove_workspace(&root).unwrap();
}

#[test]
fn missing_manifest_uses_setup_only_when_interactive() {
    let root = temp_test_path("picker-missing");
    for expected in [None, Some(crate::test_support::manifest())] {
        let setup = |_: &Utf8Path| Ok(expected.clone());
        let result = ensure(&root, None, true, &mut vec![], setup, |_, _, _| panic!()).unwrap();
        assert_eq!(result, expected);
    }
    let error = ensure(&root, None, false, &mut vec![], never, |_, _, _| panic!()).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("select the project in `niles` in a terminal"));
    remove_workspace(&root).unwrap();
}

#[test]
fn no_terminal_uses_existing_manifest_and_prints_skills() {
    let root = root("picker-noninteractive", manifest_body());
    let mut output = Vec::new();
    let result = ensure(&root, None, false, &mut output, never, |_, _, _| {
        panic!("opened picker without terminal")
    })
    .unwrap();
    assert_eq!(result, load(&root).unwrap());
    assert!(String::from_utf8(output).unwrap().contains("standard"));
    remove_workspace(&root).unwrap();
}

#[test]
fn changes_are_applied_to_latest_manifest() {
    let root = root("picker-reread", manifest_body());
    let name = uuid::Uuid::new_v4().simple().to_string();
    let selection = format!("global:{name}").parse().unwrap();
    let named = crate::skills::global_dir()
        .unwrap()
        .join("setups")
        .join(format!("{name}.yaml"));
    fs::create_dir_all(named.parent().unwrap()).unwrap();
    fs::write(&named, manifest_body()).unwrap();
    let before = super::load(&root, Some(&selection)).unwrap().unwrap();
    fs::create_dir_all(root.join(".niles/setups")).unwrap();
    fs::write(
        root.join(".niles/setups").join(format!("{name}.yaml")),
        manifest_body(),
    )
    .unwrap();
    let mut latest = before.clone();
    latest.checkin = Some("1h".to_owned());
    latest.tiers.mechanical.as_mut().unwrap().primary = "pi:z-ai/glm-5.3-flash:high".into();
    super::save(&root, &latest, Some(&selection)).unwrap();
    let mut draft = before.clone();
    draft.tiers.standard.primary = "claude:opus:medium".into();
    draft.tiers.economy.as_mut().unwrap().primary = "claude:sonnet:high".into();
    let result = save_changes(&root, Some(&selection), &before, &draft).unwrap();
    latest.tiers.standard = draft.tiers.standard;
    latest.tiers.economy = draft.tiers.economy;
    assert_eq!(result, latest);

    fs::remove_file(named).unwrap();
    remove_workspace(&root).unwrap();
}

#[test]
fn manifest_failures_do_not_open_replacement() {
    for (label, body, interactive) in [
        ("picker-malformed-terminal", Some("lead: [\n"), true),
        ("picker-malformed-no-terminal", Some("lead: [\n"), false),
        ("picker-read-failure", None, true),
    ] {
        let root = temp_test_path(label);
        let path = manifest_path(&root).unwrap();
        match body {
            Some(body) => {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, body).unwrap();
            }
            None => fs::create_dir_all(&path).unwrap(),
        }
        let error = ensure(&root, None, interactive, &mut vec![], never, |_, _, _| {
            panic!("opened picker for invalid manifest")
        })
        .unwrap_err();
        assert_eq!(error.to_string(), load(&root).unwrap_err().to_string());
        match body {
            Some(body) => assert_eq!(fs::read_to_string(&path).unwrap(), body),
            None => assert!(error.to_string().contains("failed to read"), "{error:#}"),
        }
        remove_workspace(&root).unwrap();
    }
}

#[test]
fn save_write_failure_stops_launch() {
    let root = root("picker-write-failure", manifest_body());
    let mut output = vec![];
    let error = ensure(&root, None, true, &mut output, never, |_, mut draft, _| {
        draft.tiers.standard.primary = "claude:sonnet:high".into();
        fs::remove_file(manifest_path(&root).unwrap()).unwrap();
        fs::create_dir(manifest_path(&root).unwrap()).unwrap();
        Ok(Choice::Save(Box::new(draft)))
    })
    .unwrap_err();
    assert!(error.to_string().contains("manifest.yaml"), "{error:#}");
    remove_workspace(&root).unwrap();
}

#[test]
fn secondaries_new_tiers_and_entry_agent_persist_together() {
    let root = root("picker-full-edit", manifest_body());
    let mut before = load(&root).unwrap().unwrap();
    before.tiers.frontier = None;
    before.entry_agent = before.tiers.standard.primary.clone();
    super::save(&root, &before, None).unwrap();
    let mut draft = before.clone();
    draft.tiers.standard.secondaries =
        vec!["claude:fable:medium".into(), "claude:opus:medium".into()];
    draft.tiers.set(
        Tier::Frontier,
        crate::workspace_manifest::TierBinding {
            primary: "claude:fable:high".into(),
            secondaries: vec![],
        },
    );
    draft.entry_agent = "claude:fable:medium".into();
    let saved = save_changes(&root, None, &before, &draft).unwrap();
    assert_eq!(saved, draft);
    assert_eq!(load(&root).unwrap().unwrap(), draft);
    let mut empty = draft.clone();
    empty.tiers.standard.secondaries.clear();
    empty.entry_agent = empty.tiers.standard.primary.clone();
    save_changes(&root, None, &draft, &empty).unwrap();
    assert_eq!(load(&root).unwrap().unwrap(), empty);
    remove_workspace(&root).unwrap();
}

#[test]
fn loading_deduplicates_primary_and_secondary_aliases_in_order() {
    let mut manifest = crate::test_support::manifest();
    manifest.tiers.standard.secondaries = vec![
        "codex:gpt-6.1-sol:med".into(),
        "claude:sonnet:high".into(),
        "claude:sonnet:high".into(),
        "claude:fable:medium".into(),
    ];
    let body = serde_saphyr::to_string(&manifest).unwrap();
    let root = root("dedup-existing-manifest", &body);
    let loaded = load(&root).unwrap().unwrap();
    assert_eq!(
        loaded.tiers.standard.primary,
        manifest.tiers.standard.primary
    );
    assert_eq!(
        loaded.tiers.standard.secondaries,
        ["claude:sonnet:high", "claude:fable:medium"]
    );
    assert_eq!(
        fs::read_to_string(manifest_path(&root).unwrap()).unwrap(),
        body
    );
    remove_workspace(&root).unwrap();
}
