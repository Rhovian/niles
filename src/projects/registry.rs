use std::{env, fs, os::unix::fs::symlink};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    tmux::SessionName,
    util::{read_dir_utf8_paths, utf8_path},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ProjectName(String);

impl ProjectName {
    pub fn parse(name: &str) -> Result<Self> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            bail!("project name must use ASCII letters, digits, _ or -");
        }
        Ok(Self(name.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn session(&self) -> Result<SessionName> {
        SessionName::new(self.0.clone())
    }
}

pub(super) struct Entry {
    pub name: ProjectName,
    pub path: Utf8PathBuf,
}

pub(super) fn directory() -> Result<Utf8PathBuf> {
    Ok(Utf8PathBuf::from(env::var("HOME").context("HOME is missing")?).join(".niles/projects"))
}

pub(super) fn entries() -> Result<Vec<Entry>> {
    read_dir_utf8_paths(&directory()?)?
        .into_iter()
        .filter(|link| !link.file_name().is_some_and(|name| name.starts_with('.')))
        .map(|link| {
            let name = ProjectName::parse(link.file_name().context("registry entry has no name")?)?;
            let path = utf8_path(
                fs::read_link(&link).with_context(|| format!("invalid registry link {link}"))?,
                "registry target",
            )?;
            if !path.is_absolute() {
                bail!("registry target for {} is not absolute", name.as_str());
            }
            Ok(Entry { name, path })
        })
        .collect()
}

pub(super) fn register(name: &ProjectName, path: &Utf8Path) -> std::io::Result<()> {
    let dir = directory().map_err(std::io::Error::other)?;
    fs::create_dir_all(&dir)?;
    symlink(path, dir.join(name.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_fit_tmux_sessions() {
        assert!(ProjectName::parse("api_2-prod").is_ok());
        for name in ["", "a.b", "two words", "é"] {
            assert!(ProjectName::parse(name).is_err());
        }
    }
}
