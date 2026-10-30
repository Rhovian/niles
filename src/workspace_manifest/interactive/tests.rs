use crate::agents::roster::Tier;
use std::fs;

use super::*;
fn never(_: &Utf8Path) -> Result<Option<WorkspaceManifest>> {
    panic!("setup with an existing manifest or no terminal")
}
use crate::test_support::{
    default_manifest::{load, manifest_path},
    temp_test_path,
};

fn root(label: &str, body: &str) -> camino::Utf8PathBuf {
    let root = temp_test_path(label);
    fs::create_dir_all(crate::store::paths::project_dir(&root).unwrap()).unwrap();
    fs::write(manifest_path(&root).unwrap(), body).unwrap();
    root
}

#[test]
fn changing_standard_preserves_composition_and_other_tiers() {
    let root = root(
        "picker-preserve-tiers",
        crate::test_support::manifest_body(),
    );
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
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn continue_quit_and_unchanged_save_write_nothing() {
    let root = root("picker-no-write", crate::test_support::manifest_body());
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
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            crate::test_support::manifest_body()
        );
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    }
    crate::test_support::remove_workspace(&root).unwrap();
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
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn no_terminal_uses_existing_manifest_and_prints_skills() {
    let root = root(
        "picker-noninteractive",
        crate::test_support::manifest_body(),
    );
    let mut output = Vec::new();
    let result = ensure(&root, None, false, &mut output, never, |_, _, _| {
        panic!("opened picker without terminal")
    })
    .unwrap();
    assert_eq!(result, load(&root).unwrap());
    assert!(String::from_utf8(output).unwrap().contains("standard"));
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn changes_are_applied_to_latest_manifest() {
    let root = root("picker-reread", crate::test_support::manifest_body());
    let name = uuid::Uuid::new_v4().simple().to_string();
    let selection = format!("global:{name}").parse().unwrap();
    let named = crate::skills::global_dir()
        .unwrap()
        .join("setups")
        .join(format!("{name}.yaml"));
    fs::create_dir_all(named.parent().unwrap()).unwrap();
    fs::write(&named, crate::test_support::manifest_body()).unwrap();
    let before = super::load(&root, Some(&selection)).unwrap().unwrap();
    fs::create_dir_all(root.join(".niles/setups")).unwrap();
    fs::write(
        root.join(".niles/setups").join(format!("{name}.yaml")),
        crate::test_support::manifest_body(),
    )
    .unwrap();
    let mut latest = before.clone();
    latest.checkin = Some("1h".to_owned());
    latest
        .tiers
        .set_primary(Tier::Mechanical, "pi:z-ai/glm-5.3-flash:high".into())
        .unwrap();
    super::save(&root, &latest, Some(&selection)).unwrap();
    let mut draft = before.clone();
    draft
        .tiers
        .set_primary(Tier::Standard, "claude:sonnet:high".into())
        .unwrap();
    draft
        .tiers
        .set_primary(Tier::Economy, "claude:sonnet:high".into())
        .unwrap();
    let result = save_changes(&root, Some(&selection), &before, &draft).unwrap();
    latest.tiers.standard = draft.tiers.standard;
    latest.tiers.economy = draft.tiers.economy;
    assert_eq!(result, latest);

    fs::remove_file(named).unwrap();
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn malformed_manifest_fails_without_replacement_in_any_terminal() {
    let body = "lead: [\n";
    let root = root("picker-malformed", body);
    for interactive in [true, false] {
        let error = ensure(
            &root,
            None,
            interactive,
            &mut vec![],
            never,
            |_, _, _| panic!(),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), load(&root).unwrap_err().to_string());
        assert_eq!(
            fs::read_to_string(manifest_path(&root).unwrap()).unwrap(),
            body
        );
    }
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn manifest_read_failures_do_not_open_replacement() {
    let root = temp_test_path("picker-read-failure");
    fs::create_dir_all(manifest_path(&root).unwrap()).unwrap();
    let error = ensure(&root, None, true, &mut vec![], never, |_, _, _| {
        panic!("opened picker for a read failure")
    })
    .unwrap_err();
    assert!(error.to_string().contains("failed to read"), "{error:#}");
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn save_write_failure_stops_launch() {
    let root = root("picker-write-failure", crate::test_support::manifest_body());
    let mut output = vec![];
    let error = ensure(&root, None, true, &mut output, never, |_, mut draft, _| {
        draft
            .tiers
            .set_primary(Tier::Standard, "claude:sonnet:high".into())?;
        fs::remove_file(manifest_path(&root).unwrap()).unwrap();
        fs::create_dir(manifest_path(&root).unwrap()).unwrap();
        Ok(Choice::Save(Box::new(draft)))
    })
    .unwrap_err();
    assert!(error.to_string().contains("manifest.yaml"), "{error:#}");
    crate::test_support::remove_workspace(&root).unwrap();
}
