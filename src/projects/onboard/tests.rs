use super::*;
#[test]
fn generated_defaults_match_literal_expectations() {
    for (name, description) in
        crate::store::parse_yaml::<Vec<(skills::SkillName, String)>>(defaults::SKILLS).unwrap()
    {
        assert!(!description.is_empty());
        assert_eq!(defaults::reference(&name).unwrap().rev, "HEAD");
        let path = format!("{}/skills/{name}/SKILL.md", env!("CARGO_MANIFEST_DIR"));
        assert!(Utf8Path::new(&path).exists());
    }
    let roster = crate::agents::ModelRoster::builtin().unwrap();
    for installed in "claude;codex;pi;claude codex;claude pi;codex pi;claude codex pi".split(';') {
        let generated =
            defaults::manifest(&installed.split_whitespace().collect::<Vec<_>>()).unwrap();
        manifest::check(&generated, &roster).unwrap();
        assert!(generated.bindings.is_empty());
    }
    let cases: std::collections::BTreeMap<String, String> =
        crate::store::parse_yaml(include_str!("../../test_support/onboarding.yaml")).unwrap();
    for (installed, expected) in cases {
        let generated =
            defaults::manifest(&installed.split_whitespace().collect::<Vec<_>>()).unwrap();
        let mut actual = Vec::new();
        for (tier, agents) in generated.tiers.iter() {
            let families = std::iter::once(&agents.primary)
                .chain(&agents.secondaries)
                .map(|agent| agent.split_once(':').unwrap().0)
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
    let mut picker = ui::Options::new(crate::store::parse_yaml(defaults::SKILLS).unwrap(), true);
    assert_eq!(picker.selected().count(), 0);
    picker.key(KeyCode::Char(' '));
    picker.key(KeyCode::Down);
    picker.key(KeyCode::Char(' '));
    assert_eq!(picker.selected().collect::<Vec<_>>(), ["lead", "design"]);
    picker.key(KeyCode::Char(' '));
    assert_eq!(picker.selected().collect::<Vec<_>>(), ["lead"]);
    assert_eq!(picker.key(KeyCode::Esc), Some(false));
}

#[test]
fn registration_preserves_root_and_requires_yes() {
    use ratatui::crossterm::event::KeyCode;
    use register::prompt::{Action, Prompt};
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8Path::from_path(dir.path()).unwrap();
    std::fs::create_dir(root.join("child")).unwrap();
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
        matches!(prompt.key(KeyCode::Char('y'), root).unwrap(), Action::Register { name, path } if name == "child" && path == crate::util::utf8_path(std::fs::canonicalize(root.join("child")).unwrap(), "test directory").unwrap())
    );
}

#[test]
fn entry_candidates_skip_broken_empty_and_uncached_skills_without_fetching() {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8Path::from_path(dir.path()).unwrap();
    let workspace = root.join("project");
    for (name, text) in [
        (
            "good",
            "---\nname: good\ndescription: Good instructions\n---\nCoordinate the work.\n",
        ),
        (
            "empty",
            "---\nname: empty\ndescription: No instructions\n---\n",
        ),
        ("broken", "broken YAML"),
    ] {
        let folder = root.join("skills").join(name);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("SKILL.md"), text).unwrap();
    }
    let uncached = root.join("skills/uncached");
    fs::create_dir_all(&uncached).unwrap();
    fs::write(
        uncached.join("SKILL.ref"),
        "github.com/Rhovian/niles/skills/uncached@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
    )
    .unwrap();
    let invalid = root.join("skills/invalid name");
    fs::create_dir_all(&invalid).unwrap();
    fs::write(invalid.join("SKILL.md"), "invalid").unwrap();
    let mode = ("lead".into(), "Reuse this YAML description".into());
    let (rows, warnings) = entry_skills(&mode, &workspace, root);
    assert_eq!(rows, [mode, ("good".into(), "Good instructions".into())]);
    assert_eq!(warnings.len(), 4, "{warnings:?}");
    assert!(!root.join("cache").exists());
}
