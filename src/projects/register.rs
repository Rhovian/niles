use super::registry::{self, ProjectName};
use crate::util::utf8_path;
use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use std::{fs, io::ErrorKind};

/// The directory to register for the operator's `input`, where empty means `cwd`, and the name
/// to offer for it. Errors carry the message to show before asking again.
pub(super) fn directory(input: &str, cwd: &Utf8Path) -> Result<(Utf8PathBuf, String)> {
    let candidate = if input.is_empty() {
        cwd.to_owned()
    } else {
        Utf8PathBuf::from(input)
    };
    let path = match fs::canonicalize(&candidate) {
        Ok(path) if path.is_dir() => utf8_path(path, "project path")?,
        Ok(_) => bail!("Directory does not exist: {candidate}"),
        Err(error) => bail!("Cannot open directory {candidate}: {error}"),
    };
    if let Some(existing) = registry::entries()?
        .into_iter()
        .find(|entry| entry.path == path)
    {
        bail!("already registered as {}", existing.name.as_str());
    }
    let default = path
        .file_name()
        .context("project directory has no basename")?
        .to_owned();
    Ok((path, default))
}

/// Registers `path` under `name`. Errors carry the message to show before asking again.
pub(super) fn register(name: &str, path: &Utf8Path) -> Result<()> {
    let name = ProjectName::parse(name)?;
    match registry::register(&name, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            bail!("Name {} is already taken.", name.as_str())
        }
        Err(error) => Err(error).context("failed to register project"),
    }
}
