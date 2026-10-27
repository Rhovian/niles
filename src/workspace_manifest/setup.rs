use crate::skills::{self, Origin, SkillName};
use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct SetupName(SkillName);
impl fmt::Display for SetupName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum SetupSelection {
    Unqualified(SetupName),
    Qualified(Setup),
}
impl FromStr for SetupSelection {
    type Err = anyhow::Error;
    fn from_str(text: &str) -> Result<Self> {
        match text.split_once(':') {
            None => Ok(Self::Unqualified(text.parse()?)),
            Some((origin, name)) => Ok(Self::Qualified(Setup {
                origin: origin.parse().context("invalid setup origin")?,
                name: name.parse()?,
            })),
        }
    }
}
impl FromStr for SetupName {
    type Err = anyhow::Error;
    fn from_str(name: &str) -> Result<Self> {
        Ok(Self(
            name.parse()
                .with_context(|| format!("invalid setup name {name:?}"))?,
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Setup {
    pub name: SetupName,
    pub origin: Origin,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SetupFile {
    pub setup: Setup,
    pub(super) path: Utf8PathBuf,
}
impl SetupFile {
    pub fn path(&self) -> &Utf8Path {
        &self.path
    }
}
impl fmt::Display for SetupFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({} {})",
            self.setup.name, self.setup.origin, self.path
        )
    }
}

pub(crate) enum ManifestFile {
    Default(Utf8PathBuf),
    Named(SetupFile),
}
impl ManifestFile {
    pub fn path(&self) -> &Utf8Path {
        match self {
            Self::Default(path) => path,
            Self::Named(file) => &file.path,
        }
    }
}

pub(crate) fn active(root: &Utf8Path, selection: Option<&SetupSelection>) -> Result<ManifestFile> {
    match selection {
        Some(selection) => Ok(ManifestFile::Named(
            selection.file(root, &skills::global_dir()?)?,
        )),
        None => Ok(ManifestFile::Default(
            crate::store::paths::project_dir(root)?.join("manifest.yaml"),
        )),
    }
}

impl SetupSelection {
    pub fn pin(self, root: &Utf8Path) -> Result<Self> {
        Ok(Self::Qualified(
            self.file(root, &skills::global_dir()?)?.setup,
        ))
    }
    pub(super) fn file(&self, root: &Utf8Path, global: &Utf8Path) -> Result<SetupFile> {
        let (name, selected) = match self {
            Self::Unqualified(name) => (name, None),
            Self::Qualified(setup) => (&setup.name, Some(setup.origin)),
        };
        let roots = [
            (Origin::Workspace, root.join(".niles/setups")),
            (Origin::Global, global.join("setups")),
        ];
        let mut searched = Vec::new();
        for (origin, dir) in roots
            .into_iter()
            .filter(|(origin, _)| selected.is_none_or(|selected| selected == *origin))
        {
            let path = dir.join(format!("{name}.yaml"));
            searched.push(path.to_string());
            if skills::files::exists(&path, origin)
                .with_context(|| format!("invalid setup {name} ({path})"))?
            {
                return Ok(SetupFile {
                    path,
                    setup: Setup {
                        name: name.clone(),
                        origin,
                    },
                });
            }
        }
        bail!("setup {} not found; searched {}", name, searched.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};

    struct Fixture {
        _temp: tempfile::TempDir,
        root: Utf8PathBuf,
        global: Utf8PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = Utf8PathBuf::from_path_buf(temp.path().to_owned()).unwrap();
            Self {
                global: root.join("global"),
                root,
                _temp: temp,
            }
        }
        fn path(&self, origin: Origin) -> Utf8PathBuf {
            match origin {
                Origin::Global => self.global.join("setups/tdd.yaml"),
                Origin::Workspace => self.root.join(".niles/setups/tdd.yaml"),
            }
        }
        fn select(&self, name: &str) -> Result<SetupFile> {
            name.parse::<SetupSelection>()?
                .file(&self.root, &self.global)
        }
        fn install(&self, origin: Origin) -> SetupFile {
            let path = self.path(origin);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "{}").unwrap();
            self.select(&format!("{origin}:tdd")).unwrap()
        }
    }

    #[test]
    fn missing_setup_names_every_searched_path() {
        let fixture = Fixture::new();
        let missing = fixture.select("tdd").unwrap_err().to_string();
        for origin in [Origin::Global, Origin::Workspace] {
            assert!(missing.contains(fixture.path(origin).as_str()), "{missing}");
            let qualified = fixture
                .select(&format!("{origin}:tdd"))
                .unwrap_err()
                .to_string();
            assert!(
                qualified.contains(fixture.path(origin).as_str()),
                "{qualified}"
            );
        }
    }

    #[test]
    fn workspace_wins_and_qualifiers_pin_each_root() {
        let fixture = Fixture::new();
        let global = fixture.install(Origin::Global);
        assert_eq!(fixture.select("tdd").unwrap(), global);
        let workspace = fixture.install(Origin::Workspace);
        assert_eq!(fixture.select("tdd").unwrap(), workspace);
        assert_eq!(fixture.select("global:tdd").unwrap(), global);
        assert_eq!(fixture.select("workspace:tdd").unwrap(), workspace);
    }

    #[test]
    fn save_uses_the_selected_path_without_serializing_runtime_source() {
        let fixture = Fixture::new();
        let name = uuid::Uuid::new_v4().simple().to_string();
        for origin in [Origin::Global, Origin::Workspace] {
            let path = match origin {
                Origin::Global => skills::global_dir()
                    .unwrap()
                    .join("setups")
                    .join(format!("{name}.yaml")),
                Origin::Workspace => fixture
                    .root
                    .join(".niles/setups")
                    .join(format!("{name}.yaml")),
            };
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "{}").unwrap();
            let selection: SetupSelection = format!("{origin}:{name}").parse().unwrap();
            let mut manifest = super::super::WorkspaceManifest {
                source: Some(
                    selection
                        .file(&fixture.root, &skills::global_dir().unwrap())
                        .unwrap(),
                ),
                ..crate::test_support::manifest()
            };
            manifest.bindings.insert(
                "source".parse().unwrap(),
                manifest.bindings[&manifest.entry].clone(),
            );
            super::super::save(&fixture.root, &manifest, Some(&selection)).unwrap();
            let saved = crate::store::parse_yaml::<super::super::WorkspaceManifest>(
                &fs::read_to_string(&path).unwrap(),
            )
            .unwrap();
            assert_eq!(saved.bindings, manifest.bindings, "{origin}");
            fs::remove_file(path).unwrap();
        }
        assert!(!active(&fixture.root, None).unwrap().path().exists());
    }

    #[test]
    fn workspace_setup_rejects_symlinked_file_and_directory_with_setup_context() {
        for directory in [false, true] {
            let fixture = Fixture::new();
            let target = fixture.install(Origin::Global);
            let path = fixture.path(Origin::Workspace);
            fs::create_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
            if directory {
                symlink(target.path.parent().unwrap(), path.parent().unwrap()).unwrap();
            } else {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                symlink(target.path, &path).unwrap();
            }
            let error = format!("{:#}", fixture.select("workspace:tdd").unwrap_err());
            assert!(
                error.contains("setup tdd") && error.contains("without symlinks"),
                "directory={directory}: {error}"
            );
        }
    }
}
