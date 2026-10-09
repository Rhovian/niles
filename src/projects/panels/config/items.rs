//! The CONFIG panel's rows: each key with its value and the file that sets it.

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    agents::ModelRoster,
    config::{
        spec::{
            PROJECT_CONFIG_FILES, ProjectConfig, load_project_config_from, project_config_file,
        },
        user::{self, DEFAULT_BINDINGS, DEFAULT_THEME, FileConfig},
    },
    projects::registry,
    watch,
    workspace_manifest::{
        self, AgentGroup, MISSING, RoleBinding, WorkspaceManifest, clamp, manifest_roles,
    },
};

const BUILTIN: &str = "builtin";
const MANIFEST: &str = "manifest";
const CONFIG_YAML: &str = "config.yaml";
const REGISTRY: &str = "registry";
/// Repo files are arbitrary text, so every value shown from one is clamped to one inert cell.
const VALUE_WIDTH: usize = 60;
const REASON_WIDTH: usize = 68;

pub(super) enum Item {
    Section(String),
    Setting(Setting),
    /// A file that failed to load, in place of the settings it holds.
    Broken {
        reason: String,
        edit: Edit,
    },
}

impl Item {
    /// What ↵ does on this row; a section heading does nothing.
    pub(super) fn edit(&self) -> Option<&Edit> {
        match self {
            Self::Section(_) => None,
            Self::Setting(setting) => Some(&setting.edit),
            Self::Broken { edit, .. } => Some(edit),
        }
    }
}

pub(super) struct Setting {
    pub key: String,
    pub value: String,
    /// `builtin` when no file sets the key, else the file that does.
    pub from: &'static str,
    pub note: Note,
    pub edit: Edit,
}

pub(super) enum Note {
    None,
    /// The builtin value a file overrides.
    Builtin(String),
    Invalid(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Edit {
    /// Runs the role's picker, then saves only that role into the manifest at `root`.
    Role {
        root: Utf8PathBuf,
        role: Role,
    },
    File(Utf8PathBuf),
    /// The registry is a directory of links, changed from the explorer rather than a file.
    Registry {
        name: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    Lead,
    Worker,
    Reviewer,
    Security,
}

impl Role {
    /// In the order `manifest_roles` lists them.
    const ALL: [Self; 4] = [Self::Lead, Self::Worker, Self::Reviewer, Self::Security];
}

pub(super) fn global(config: &Utf8Path, registry: &[registry::Entry]) -> Vec<Item> {
    let edit = Edit::File(config.to_owned());
    let mut items = vec![Item::Section(CONFIG_YAML.to_owned())];
    match user::read(config).and_then(|text| FileConfig::parse(text.as_deref())) {
        Ok(file) => items.extend(
            [
                setting(
                    "theme",
                    file.theme.map(|theme| theme.slug().to_owned()),
                    DEFAULT_THEME.slug(),
                    CONFIG_YAML,
                    &edit,
                ),
                setting(
                    "tmux.bindings",
                    file.tmux.bindings.map(|bindings| bindings.to_string()),
                    &DEFAULT_BINDINGS.to_string(),
                    CONFIG_YAML,
                    &edit,
                ),
            ]
            .map(Item::Setting),
        ),
        Err(error) => items.push(broken(&error, edit)),
    }
    items.push(Item::Section(REGISTRY.to_owned()));
    items.extend(registry.iter().map(|entry| {
        Item::Setting(Setting {
            key: entry.name.as_str().to_owned(),
            value: clamp(entry.path.as_str(), VALUE_WIDTH),
            from: REGISTRY,
            note: Note::None,
            edit: Edit::Registry {
                name: entry.name.as_str().to_owned(),
            },
        })
    }));
    items
}

pub(super) fn project(root: &Utf8Path) -> Result<Vec<Item>> {
    let manifest_edit = Edit::File(workspace_manifest::manifest_path(root));
    let file = match project_config_file(root)? {
        Some(file) => file,
        None => PROJECT_CONFIG_FILES[0],
    };
    let file_edit = Edit::File(root.join(file));
    let config = load_project_config_from(root);
    let mut items = Vec::new();
    match workspace_manifest::load(root) {
        Ok(Some(manifest)) => {
            match &config {
                Ok(config) => items.extend(roles(root, &manifest, config, &manifest_edit)),
                Err(error) => items.extend([
                    Item::Section("roles".to_owned()),
                    broken(error, file_edit.clone()),
                ]),
            }
            items.extend(watch(&manifest, &manifest_edit)?);
            items.push(Item::Section("worker_planning".to_owned()));
            items.extend(listed(
                manifest.worker_planning.0.iter().map(|group| {
                    plain(
                        &group.models.join(", "),
                        &group.guidance.lines().take(1).collect::<String>(),
                        MANIFEST,
                        &manifest_edit,
                    )
                }),
                &manifest_edit,
            ));
        }
        Ok(None) => items.extend([
            Item::Section(MANIFEST.to_owned()),
            Item::Broken {
                reason: ".niles/manifest.yaml".to_owned(),
                edit: manifest_edit,
            },
        ]),
        Err(error) => items.extend([
            Item::Section(MANIFEST.to_owned()),
            broken(&error, manifest_edit),
        ]),
    }
    match &config {
        Ok(config) => items.extend(files(config, file, &file_edit)?),
        Err(error) => items.extend([Item::Section(file.to_owned()), broken(error, file_edit)]),
    }
    Ok(items)
}

fn roles(
    root: &Utf8Path,
    manifest: &WorkspaceManifest,
    config: &ProjectConfig,
    manifest_edit: &Edit,
) -> Vec<Item> {
    let mut items = Vec::new();
    for (role, row) in Role::ALL.into_iter().zip(manifest_roles(manifest, config)) {
        items.push(Item::Section(format!("roles.{}", row.role)));
        let mut invalid = row.invalid_reason.map(Note::Invalid);
        let groups = match role {
            Role::Lead => None,
            Role::Worker => groups(&manifest.worker),
            Role::Reviewer => manifest.reviewer.as_agent().and_then(groups),
            Role::Security => groups(&manifest.security),
        };
        let Some(groups) = groups else {
            let edit = Edit::Role {
                root: root.to_owned(),
                role,
            };
            items.extend(
                [
                    ("agent", row.family),
                    ("model", row.model),
                    ("effort", row.effort),
                ]
                .map(|(key, value)| {
                    let from = if value == MISSING { BUILTIN } else { MANIFEST };
                    let mut setting = plain(key, &value, from, &edit);
                    if let Some(note) = invalid.take() {
                        setting.note = note;
                    }
                    Item::Setting(setting)
                }),
            );
            continue;
        };
        items.extend(groups.iter().enumerate().map(|(index, group)| {
            let models = group.models.join(", ");
            let value = match &group.efforts {
                Some(efforts) => format!("{models} [{}]", efforts.join(", ")),
                None => models,
            };
            let key = match &group.when {
                Some(when) => when.clone(),
                None => format!("group {}", index + 1),
            };
            let mut setting = plain(&key, &value, MANIFEST, manifest_edit);
            if let Some(note) = invalid.take() {
                setting.note = note;
            }
            Item::Setting(setting)
        }));
    }
    items
}

/// A role written as groups, which only the editor can change; `None` for one model.
fn groups(binding: &RoleBinding) -> Option<&[AgentGroup]> {
    binding.scalar().is_none().then_some(binding.0.as_slice())
}

fn watch(manifest: &WorkspaceManifest, edit: &Edit) -> Result<[Item; 3]> {
    let builtin = watch::resolve_cadence(None, None, Utf8Path::new(""))?;
    let checkin_builtin = match builtin.delay {
        Some(delay) => watch::describe_delay(delay),
        None => "off".to_owned(),
    };
    let mut checkin = setting(
        "checkin",
        manifest.checkin.clone(),
        &checkin_builtin,
        MANIFEST,
        edit,
    );
    if let Some(Err(error)) = manifest.checkin.as_deref().map(watch::parse_delay) {
        checkin.note = invalid(&error);
    }
    let mut recheck = setting(
        "recheck",
        manifest.recheck.clone(),
        &builtin.recheck.spelling(),
        MANIFEST,
        edit,
    );
    if let Some(Err(error)) = manifest.recheck.as_deref().map(watch::parse_recheck) {
        recheck.note = invalid(&error);
    }
    Ok([
        Item::Section("watch".to_owned()),
        Item::Setting(checkin),
        Item::Setting(recheck),
    ])
}

/// The `niles.yaml` sections: its custom agents, and the models it adds or changes.
fn files(config: &ProjectConfig, file: &'static str, edit: &Edit) -> Result<Vec<Item>> {
    let builtin = ModelRoster::builtin()?.rows();
    let agents = config.agents.iter().map(|(name, agent)| {
        let command = agent.binary.iter().chain(&agent.args);
        plain(
            name,
            &command.cloned().collect::<Vec<_>>().join(" "),
            file,
            edit,
        )
    });
    let mut items = vec![Item::Section("agents".to_owned())];
    items.extend(listed(agents, edit));
    items.push(Item::Section("models".to_owned()));
    let models = config
        .models
        .rows()
        .into_iter()
        .filter(|row| !builtin.contains(row));
    let models = models.map(|[family, model, efforts]| {
        let mut setting = plain(&format!("{family}:{model}"), &efforts, file, edit);
        if let Some([.., builtin]) = builtin.iter().find(|[f, m, _]| *f == family && *m == model) {
            setting.note = Note::Builtin(builtin.clone());
        }
        setting
    });
    items.extend(listed(models, edit));
    Ok(items)
}

/// A section's settings, or one builtin `-` row so an empty section still has a row to edit.
fn listed(settings: impl Iterator<Item = Setting>, edit: &Edit) -> Vec<Item> {
    let mut items = settings.map(Item::Setting).collect::<Vec<_>>();
    if items.is_empty() {
        items.push(Item::Setting(plain(MISSING, MISSING, BUILTIN, edit)));
    }
    items
}

/// A key with a builtin value: the file's value with that builtin noted, else the builtin.
fn setting(
    key: &str,
    set: Option<String>,
    builtin: &str,
    from: &'static str,
    edit: &Edit,
) -> Setting {
    match set {
        Some(value) => Setting {
            note: Note::Builtin(builtin.to_owned()),
            ..plain(key, &value, from, edit)
        },
        None => plain(key, builtin, BUILTIN, edit),
    }
}

fn plain(key: &str, value: &str, from: &'static str, edit: &Edit) -> Setting {
    Setting {
        key: clamp(key, VALUE_WIDTH),
        value: clamp(value, VALUE_WIDTH),
        from,
        note: Note::None,
        edit: edit.clone(),
    }
}

fn invalid(error: &anyhow::Error) -> Note {
    Note::Invalid(clamp(&format!("{error:#}"), REASON_WIDTH))
}

/// The section names the file, so its root cause is the part of the error that fits.
fn broken(error: &anyhow::Error, edit: Edit) -> Item {
    Item::Broken {
        reason: clamp(&error.root_cause().to_string(), REASON_WIDTH),
        edit,
    }
}
