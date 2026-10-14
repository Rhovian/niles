use std::fs;

use camino::Utf8PathBuf;

use super::*;
use crate::test_support::temp_test_path;
use crate::workspace_manifest::RoleBinding;

/// Each row as `key | value | from | note`, so a whole scope reads as one table.
fn table(items: &[Item]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Section(name) => format!("[{name}]"),
            Item::Setting(setting) => {
                let note = match &setting.note {
                    Note::None => String::new(),
                    Note::Builtin(value) => format!("builtin: {value}"),
                    Note::Invalid(reason) => format!("✗ {reason}"),
                };
                let row = [&setting.key, &setting.value, setting.from, &note];
                row.map(|cell| cell.to_string()).join(" | ")
            }
            Item::Broken(reason) => format!("✗ {reason}"),
        })
        .collect()
}

/// Invalid reasons name the value and the rule; the expected row is that reason's start.
fn assert_rows(items: &[Item], expected: &[&str]) {
    let actual = table(items);
    assert_eq!(actual.len(), expected.len(), "{actual:#?}");
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(
            actual.starts_with(expected),
            "{actual:?} is not {expected:?}"
        );
    }
}

#[test]
fn global_rows_show_builtin_overridden_and_invalid_config() {
    let dir = temp_test_path("config-global");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.yaml");
    let registry = [registry::Entry {
        name: ProjectName::parse("api").unwrap(),
        path: "/w/api".into(),
    }];
    assert_rows(
        &items::global(&path, &registry),
        &[
            "[config.yaml]",
            "theme | tokyo-night | builtin | ",
            "tmux.bindings | false | builtin | ",
            "[registry]",
            "api | /w/api | registry | ",
        ],
    );
    fs::write(&path, "theme: nord\ntmux: {bindings: false}\n").unwrap();
    assert_rows(
        &items::global(&path, &[]),
        &[
            "[config.yaml]",
            "theme | nord | config.yaml | builtin: tokyo-night",
            "tmux.bindings | false | config.yaml | builtin: false",
            "[registry]",
        ],
    );
    fs::write(&path, "theme: nord\nbogus: 1\n").unwrap();
    let items = items::global(&path, &[]);
    assert_rows(&items, &["[config.yaml]", "✗ ", "[registry]"]);
    assert!(table(&items)[1].contains("bogus"), "{:?}", table(&items));
    assert_eq!(items[1].edit(), None);
    fs::remove_dir_all(dir).unwrap();
}

const MANIFEST: &str = include_str!("../../../../examples/claude-lead.yaml");

fn project(label: &str, manifest: Option<&str>, file: Option<&str>) -> Utf8PathBuf {
    let root = temp_test_path(label);
    fs::create_dir_all(root.join(".niles")).unwrap();
    if let Some(manifest) = manifest {
        fs::write(workspace_manifest::manifest_path(&root), manifest).unwrap();
    }
    if let Some(file) = file {
        fs::write(root.join(".niles.yaml"), file).unwrap();
    }
    root
}

#[test]
fn project_rows_show_tiers_and_standard_primary_is_editable() {
    let root = project("config-project-tiers", Some(MANIFEST), None);
    let items = items::project(&root).unwrap();
    let rows = table(&items);
    assert!(rows.iter().any(|row| {
        row.starts_with("standard | claude:sonnet:high; secondaries: codex:gpt-6.1-sol:medium")
    }));
    assert!(rows.iter().any(|row| {
        row.starts_with("mechanical | codex:gpt-6-luna:low; secondaries: claude:haiku")
    }));
    assert_eq!(
        items[1].edit(),
        Some(&Edit::Role {
            root: root.clone(),
            role: Role::Lead
        })
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_manifest_and_invalid_project_file_show_in_their_sections() {
    let root = project("config-missing", None, None);
    let items = items::project(&root).unwrap();
    assert_rows(
        &items,
        &[
            "[manifest]",
            "✗ .niles/manifest.yaml",
            "[agents]",
            "- | - | builtin | ",
            "[models]",
            "- | - | builtin | ",
        ],
    );
    for index in [1, 3, 5] {
        assert_eq!(items[index].edit(), None);
    }
    fs::remove_dir_all(root).unwrap();

    let root = project(
        "config-invalid",
        Some(MANIFEST),
        Some("agents: {lead: {}}\n"),
    );
    let rows = table(&items::project(&root).unwrap());
    assert_eq!(rows[0], "[manifest]");
    assert!(
        rows[1].contains("custom agent name `lead` is reserved"),
        "{rows:?}"
    );
    assert_eq!(rows[rows.len() - 2], "[.niles.yaml]");
    assert!(
        rows[rows.len() - 1].contains("`lead` is reserved"),
        "{rows:?}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn saving_a_role_and_checkin_keeps_the_other_fields() {
    let text = format!("{MANIFEST}checkin: 300s\nrecheck: backoff\n");
    let root = project("config-save", Some(&text), None);
    let before = workspace_manifest::load(&root).unwrap().unwrap();
    save_manifest(&root, |manifest| {
        manifest.security = RoleBinding::from("codex:gpt-5.5:high".to_owned());
    })
    .unwrap();
    let rows = items::project(&root).unwrap();
    let Item::Setting(setting) = rows
        .iter()
        .find(|item| matches!(item, Item::Setting(setting) if setting.key == "checkin"))
        .unwrap()
    else {
        panic!("missing checkin row")
    };
    let Some(Edit::Step(step)) = &setting.edit else {
        panic!("missing checkin step")
    };
    step.save(cycle(&step.options(), &setting.value, false))
        .unwrap();
    let after = workspace_manifest::load(&root).unwrap().unwrap();
    assert_eq!(
        after,
        WorkspaceManifest {
            security: RoleBinding::from("codex:gpt-5.5:high".to_owned()),
            checkin: Some("off".to_owned()),
            ..before
        }
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stepping_theme_creates_sparse_config_and_reloads_rows() {
    let dir = temp_test_path("config-theme");
    let path = dir.join(".niles/config.yaml");
    let step = Step::Theme(path.clone());
    let rows = items::global(&path, &[]);
    let Item::Setting(setting) = &rows[1] else {
        panic!("missing theme row")
    };
    step.save(cycle(&step.options(), &setting.value, false))
        .unwrap();
    let next = ThemeName::SolarizedDark;
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!("theme: {}\n", next.slug())
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cycle_wraps_and_starts_at_an_end_when_unmatched() {
    let options = ["one", "two", "three"];
    assert_eq!(cycle(&options, "three", false), "one");
    assert_eq!(cycle(&options, "one", true), "three");
    assert_eq!(cycle(&options, "unknown", false), "one");
    assert_eq!(cycle(&options, "unknown", true), "three");
}
