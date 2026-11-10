use super::*;
use crate::agents::ModelRoster;
use crate::store::{self, atomic_write};
use crate::util;
use std::collections::BTreeMap;
use std::{fs, iter};
#[test]
fn generated_defaults_match_literal_expectations() {
    for (name, description) in
        store::parse_yaml::<Vec<(skills::SkillName, String)>>(defaults::SKILLS).unwrap()
    {
        assert!(!description.is_empty());
        assert_eq!(defaults::reference(&name).unwrap().rev, "HEAD");
        let path = format!("{}/skills/{name}/SKILL.md", env!("CARGO_MANIFEST_DIR"));
        assert!(Utf8Path::new(&path).exists());
    }
    let roster = ModelRoster::builtin().unwrap();
    let cases: BTreeMap<String, String> =
        store::parse_yaml(include_str!("../../test_support/onboarding.yaml")).unwrap();
    for (installed, expected) in cases {
        let generated =
            defaults::manifest(&installed.split_whitespace().collect::<Vec<_>>()).unwrap();
        manifest::check(&generated, &roster).unwrap();
        assert!(generated.bindings.is_empty());
        let mut actual = Vec::new();
        for (tier, agents) in generated.tiers.iter() {
            let families = iter::once(&agents.primary)
                .chain(&agents.secondaries)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(",");
            actual.push(format!("{}={families}", tier.as_str()));
        }
        assert_eq!(actual.join("\n") + "\n", expected, "{installed}");
    }
}

#[test]
fn starter_picker_starts_empty_and_toggles_independently() {
    use ratatui::crossterm::event::KeyCode;
    let mut picker = ui::Options::new(store::parse_yaml(defaults::SKILLS).unwrap(), true).unwrap();
    assert_eq!(picker.selected().count(), 0);
    picker.key(KeyCode::Char(' '));
    picker.key(KeyCode::Down);
    picker.key(KeyCode::Char(' '));
    assert_eq!(picker.selected().collect::<Vec<_>>(), ["lead", "design"]);
    picker.key(KeyCode::Char(' '));
    assert_eq!(picker.selected().collect::<Vec<_>>(), ["lead"]);
    assert_eq!(picker.key(KeyCode::Esc), Some(ui::Action::Back));
}

#[test]
fn registration_preserves_root_and_requires_yes() {
    use ratatui::crossterm::event::KeyCode;
    use register::prompt::{Action, Prompt};
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8Path::from_path(dir.path()).unwrap();
    fs::create_dir(root.join("child")).unwrap();
    let mut prompt = Prompt::new();
    prompt.key(KeyCode::Backspace, root).unwrap();
    assert_eq!(prompt.text(root), format!("directory: {root}/"));
    for c in "child".chars() {
        prompt.key(KeyCode::Char(c), root).unwrap();
    }
    prompt.key(KeyCode::Enter, root).unwrap();
    assert!(prompt.text(root).contains("name [child]"));
    prompt.key(KeyCode::Enter, root).unwrap();
    assert!(prompt.text(root).contains("Register? y/N"));
    for key in [KeyCode::Enter, KeyCode::Char('N'), KeyCode::Esc] {
        assert!(matches!(prompt.key(key, root).unwrap(), Action::Cancel));
    }
    assert!(
        matches!(prompt.key(KeyCode::Char('y'), root).unwrap(), Action::Register { name, path } if name == "child" && path == util::utf8_path(fs::canonicalize(root.join("child")).unwrap(), "test directory").unwrap())
    );
}

#[test]
fn custom_refs_use_import_grammar_and_refuse_existing_names_without_fetching() {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8Path::from_path(dir.path()).unwrap();
    for text in [
        "https://github.com/o/r/skill",
        "gitlab.com/o/r/skill",
        "invalid",
        "github.com/o/r/../skill",
    ] {
        assert!(custom::reference(text, root, root).is_err(), "{text}");
    }
    let reference = custom::reference("github.com/o/r/skills/custom@branch", root, root).unwrap();
    assert_eq!(reference.name.as_str(), "custom");
    assert_eq!(reference.rev, "branch");
    fs::create_dir_all(root.join("skills/custom")).unwrap();
    atomic_write(
        root.join("skills/custom/SKILL.md"),
        "broken unrelated skill",
    )
    .unwrap();
    let error = custom::reference(&reference.to_string(), root, root).unwrap_err();
    assert!(error.to_string().contains("already installed"));
    assert!(!root.join("cache").exists());
}
