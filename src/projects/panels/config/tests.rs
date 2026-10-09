use std::{fs, os::unix::fs::PermissionsExt};

use camino::Utf8PathBuf;

use super::*;
use crate::{agents::ModelRoster, test_support::temp_test_path};

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
            Item::Broken { reason, .. } => format!("✗ {reason}"),
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

fn entry(name: &str, path: &str) -> registry::Entry {
    registry::Entry {
        name: ProjectName::parse(name).unwrap(),
        path: path.into(),
    }
}

#[test]
fn global_rows_show_builtin_overridden_and_invalid_config() {
    let dir = temp_test_path("config-global");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.yaml");
    let registry = [entry("api", "/w/api")];
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
    assert_eq!(items[1].edit(), Some(&Edit::File(path)));
    fs::remove_dir_all(dir).unwrap();
}

const MANIFEST: &str = "\
lead: claude:opus:max
worker:
  - when: settled plan
    models: [codex:gpt-5.5]
    efforts: [medium, high]
  - models: [claude:opus]
reviewer: ghost
security: claude
checkin: 15m
recheck: nope
worker_planning:
  - models: [codex:gpt-5.5, claude:opus]
    guidance: |
      Plan lightly.
      Then hand off.
";

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
fn project_rows_show_builtin_overridden_multi_group_and_invalid_values() {
    let root = project(
        "config-project",
        Some(MANIFEST),
        Some(
            "agents: {bot: {binary: bot, args: [--fast]}}\nmodels: {codex: {gpt-5.5: {efforts: [high]}}}\n",
        ),
    );
    let builtin = ModelRoster::builtin().unwrap();
    let efforts = builtin
        .supported_efforts("codex", "gpt-5.5")
        .unwrap()
        .join(" ");
    let items = items::project(&root).unwrap();
    assert_rows(
        &items,
        &[
            "[roles.lead]",
            "agent | claude | manifest | ",
            "model | opus | manifest | ",
            "effort | max | manifest | ",
            "[roles.worker]",
            "settled plan | codex:gpt-5.5 [medium, high] | manifest | ✗ model gpt-5.5 does not support listed effort medium",
            "group 2 | claude:opus | manifest | ",
            "[roles.reviewer]",
            "agent | ghost | manifest | ✗ unknown agent `ghost`",
            "model | - | builtin | ",
            "effort | - | builtin | ",
            "[roles.security]",
            "agent | claude | manifest | ",
            "model | - | builtin | ",
            "effort | - | builtin | ",
            "[watch]",
            "checkin | 15m | manifest | builtin: 5m",
            "recheck | nope | manifest | ✗ `nope` is not a duration",
            "[worker_planning]",
            "codex:gpt-5.5, claude:opus | Plan lightly. | manifest | ",
            "[agents]",
            "bot | bot --fast | .niles.yaml | ",
            "[models]",
            &format!("codex:gpt-5.5 | high | .niles.yaml | builtin: {efforts}"),
        ],
    );
    let manifest = Edit::File(workspace_manifest::manifest_path(&root));
    let role = |role| Edit::Role {
        root: root.clone(),
        role,
    };
    assert_eq!(items[1].edit(), Some(&role(Role::Lead)));
    assert_eq!(items[5].edit(), Some(&manifest));
    assert_eq!(items[8].edit(), Some(&role(Role::Reviewer)));
    assert_eq!(items[17].edit(), Some(&manifest));
    assert_eq!(
        items[21].edit(),
        Some(&Edit::File(root.join(".niles.yaml")))
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
    assert_eq!(
        items[1].edit(),
        Some(&Edit::File(workspace_manifest::manifest_path(&root)))
    );
    // With no project file, its sections edit the canonical one.
    assert_eq!(items[3].edit(), Some(&Edit::File(root.join("niles.yaml"))));
    fs::remove_dir_all(root).unwrap();

    let root = project(
        "config-invalid",
        Some(MANIFEST),
        Some("agents: {lead: {}}\n"),
    );
    let rows = table(&items::project(&root).unwrap());
    assert_eq!(rows[0], "[roles]");
    assert!(
        rows[1].starts_with("✗ custom agent name `lead` is reserved"),
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
fn saving_a_role_keeps_the_other_roles_and_planning() {
    let root = project("config-save", Some(MANIFEST), None);
    let before = workspace_manifest::load(&root).unwrap().unwrap();
    save_role(&root, |manifest| {
        manifest.security = RoleBinding::from("codex:gpt-5.5:high".to_owned());
    })
    .unwrap();
    let after = workspace_manifest::load(&root).unwrap().unwrap();
    assert_eq!(
        after,
        WorkspaceManifest {
            security: RoleBinding::from("codex:gpt-5.5:high".to_owned()),
            ..before
        }
    );
    assert!(!after.worker_planning.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn editor_edits_the_file_and_the_rows_reload() {
    let dir = temp_test_path("config-editor");
    fs::create_dir_all(&dir).unwrap();
    let editor = dir.join("editor");
    fs::write(&editor, "#!/bin/sh\nprintf 'theme: nord\\n' >> \"$1\"\n").unwrap();
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    let path = dir.join("config.yaml");
    fs::write(&path, "# kept\n").unwrap();
    edit_file(Some(editor.as_os_str()), &path).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "# kept\ntheme: nord\n");
    assert_eq!(
        table(&items::global(&path, &[]))[1],
        "theme | nord | config.yaml | builtin: tokyo-night"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unset_editor_names_the_file_and_changes_nothing() {
    let dir = temp_test_path("config-no-editor");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.yaml");
    fs::write(&path, "theme: nord\n").unwrap();
    let error = edit_file(None, &path).unwrap_err();
    assert_eq!(error.to_string(), format!("set $EDITOR to edit {path}"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "theme: nord\n");
    fs::remove_dir_all(dir).unwrap();
}
