//! The CONFIG panel's rows: each key with its value and the file that sets it.

use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    agents::ModelCatalogue,
    config::user::{DEFAULT_BINDINGS, DEFAULT_THEME, FileConfig},
    projects::registry,
    watch,
    workspace_manifest::{self, MISSING, WorkspaceManifest, clamp, manifest_tiers},
};

const CLI: &str = "CLI";
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
    /// A source diagnostic alongside sections that remain available.
    Broken(String),
}

impl Item {
    /// What ↵ does on this row; a section heading does nothing.
    pub(super) fn edit(&self) -> Option<&Edit> {
        match self {
            Self::Section(_) | Self::Broken(_) => None,
            Self::Setting(setting) => setting.edit.as_ref(),
        }
    }
}

pub(super) struct Setting {
    pub key: String,
    pub value: String,
    /// `builtin` when no file sets the key, else the file that does.
    pub from: &'static str,
    pub note: Note,
    pub edit: Option<Edit>,
}

pub(super) enum Note {
    None,
    /// The builtin value a file overrides.
    Builtin(String),
    Invalid(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Edit {
    /// Runs the tier's picker, then saves only that tier into the manifest at `root`.
    Tier {
        root: Utf8PathBuf,
        tier: Tier,
    },
    Step(Step),
    /// The registry is a directory of links, changed from the explorer rather than a file.
    Registry,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Theme(Utf8PathBuf),
    Bindings(Utf8PathBuf),
    Checkin(Utf8PathBuf),
    Recheck(Utf8PathBuf),
}

pub(super) use crate::workspace_manifest::Tier;

pub(super) fn global(config: &Utf8Path, registry: &[registry::Entry]) -> Vec<Item> {
    let mut items = vec![Item::Section(CONFIG_YAML.to_owned())];
    match FileConfig::load(config) {
        Ok(file) => items.extend(
            [
                setting(
                    "theme",
                    file.theme.map(|theme| theme.slug().to_owned()),
                    DEFAULT_THEME.slug(),
                    CONFIG_YAML,
                    &Edit::Step(Step::Theme(config.to_owned())),
                ),
                setting(
                    "tmux.bindings",
                    file.tmux.bindings.map(|bindings| bindings.to_string()),
                    &DEFAULT_BINDINGS.to_string(),
                    CONFIG_YAML,
                    &Edit::Step(Step::Bindings(config.to_owned())),
                ),
            ]
            .map(Item::Setting),
        ),
        Err(error) => items.push(broken(&error)),
    }
    items.push(Item::Section(REGISTRY.to_owned()));
    items.extend(registry.iter().map(|entry| {
        Item::Setting(Setting {
            key: entry.name.as_str().to_owned(),
            value: clamp(entry.path.as_str(), VALUE_WIDTH),
            from: REGISTRY,
            note: Note::None,
            edit: Some(Edit::Registry),
        })
    }));
    items
}

pub(super) fn project(root: &Utf8Path, catalogue: &ModelCatalogue) -> Vec<Item> {
    let mut items = Vec::new();
    match workspace_manifest::load(root, None) {
        Ok(Some(manifest)) => {
            items.extend(tiers(root, &manifest));
            items.extend(watch(root, &manifest));
        }
        Ok(None) => items.extend([
            Item::Section(MANIFEST.to_owned()),
            Item::Broken("global manifest.yaml".to_owned()),
        ]),
        Err(error) => items.extend([Item::Section(MANIFEST.to_owned()), broken(&error)]),
    }
    items.extend(models(catalogue));
    items
}

fn tiers(root: &Utf8Path, manifest: &WorkspaceManifest) -> Vec<Item> {
    let mut items = Vec::new();
    for row in manifest_tiers(manifest) {
        items.push(Item::Section(format!("tiers.{}", row.tier.as_str())));
        let mut invalid = row.invalid_reason.map(Note::Invalid);
        let edit = Some(Edit::Tier {
            root: root.to_owned(),
            tier: row.tier,
        });
        items.extend(
            [
                ("agent", row.family),
                ("model", row.model),
                ("effort", row.effort),
            ]
            .map(|(key, value)| {
                let from = if value == MISSING { BUILTIN } else { MANIFEST };
                let mut setting = plain(key, &value, from);
                setting.edit = edit.clone();
                if let Some(note) = invalid.take() {
                    setting.note = note;
                }
                Item::Setting(setting)
            }),
        );
        items.push(Item::Setting(plain(
            "secondaries",
            &row.secondaries,
            MANIFEST,
        )));
    }
    items
}

fn watch(root: &Utf8Path, manifest: &WorkspaceManifest) -> [Item; 3] {
    let mut checkin = setting(
        "checkin",
        manifest.checkin.clone(),
        &watch::describe_delay(watch::DEFAULT_DELAY),
        MANIFEST,
        &Edit::Step(Step::Checkin(root.to_owned())),
    );
    if let Some(Err(error)) = manifest.checkin.as_deref().map(watch::parse_delay) {
        checkin.note = invalid(&error);
    }
    let mut recheck = setting(
        "recheck",
        manifest.recheck.clone(),
        &watch::Recheck::Backoff.spelling(),
        MANIFEST,
        &Edit::Step(Step::Recheck(root.to_owned())),
    );
    if let Some(Err(error)) = manifest.recheck.as_deref().map(watch::parse_recheck) {
        recheck.note = invalid(&error);
    }
    [
        Item::Section("watch".to_owned()),
        Item::Setting(checkin),
        Item::Setting(recheck),
    ]
}

/// Installed CLIs supply the available models and effort levels.
fn models(catalogue: &ModelCatalogue) -> Vec<Item> {
    let mut items = vec![Item::Section("models".to_owned())];
    for profile in crate::agents::profiles() {
        match catalogue.models(profile.id) {
            Ok(models) => items.extend(models.map(|(model, efforts)| {
                Item::Setting(plain(
                    &format!("{}:{model}", profile.id),
                    &efforts.join(" "),
                    CLI,
                ))
            })),
            Err(error) => items.push(broken(&error)),
        }
    }
    if items.len() == 1 {
        items.push(Item::Setting(plain(MISSING, MISSING, BUILTIN)));
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
    let mut setting = match set {
        Some(value) => Setting {
            note: Note::Builtin(builtin.to_owned()),
            ..plain(key, &value, from)
        },
        None => plain(key, builtin, BUILTIN),
    };
    setting.edit = Some(edit.clone());
    setting
}

fn plain(key: &str, value: &str, from: &'static str) -> Setting {
    Setting {
        key: clamp(key, VALUE_WIDTH),
        value: clamp(value, VALUE_WIDTH),
        from,
        note: Note::None,
        edit: None,
    }
}

fn invalid(error: &anyhow::Error) -> Note {
    Note::Invalid(clamp(&format!("{error:#}"), REASON_WIDTH))
}

/// The section names the file, so its root cause is the part of the error that fits.
fn broken(error: &anyhow::Error) -> Item {
    Item::Broken(clamp(&error.root_cause().to_string(), REASON_WIDTH))
}
