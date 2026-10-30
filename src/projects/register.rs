use super::registry::{self, ProjectName};
use crate::util::utf8_path;
use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use std::{fs, io::ErrorKind};

pub(super) enum Directory {
    Registered(registry::Entry),
    Unregistered { path: Utf8PathBuf, default: String },
}

/// Resolve `input` (empty means `cwd`) to its canonical registration or suggested project name.
pub(super) fn directory(input: &str, cwd: &Utf8Path) -> Result<Directory> {
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
        return Ok(Directory::Registered(existing));
    }
    let default = path
        .file_name()
        .context("project directory has no basename")?
        .to_owned();
    Ok(Directory::Unregistered { path, default })
}

/// Registers `path` under `name`. Errors carry the message to show before asking again.
pub(super) fn register(name: &str, path: &Utf8Path) -> Result<registry::Entry> {
    let name = ProjectName::parse(name)?;
    match registry::register(&name, path) {
        Ok(()) => Ok(registry::Entry {
            name,
            path: path.to_owned(),
        }),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            bail!("Name {} is already taken.", name.as_str())
        }
        Err(error) => Err(error).context("failed to register project"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn taken_name_is_retryable() {
        let name = uuid::Uuid::new_v4().simple().to_string();
        let path = crate::util::home_dir().unwrap();
        register(&name, &path).unwrap();
        let error = register(&name, &path).err().unwrap();
        assert!(error.downcast_ref::<std::io::Error>().is_none());
        assert!(error.to_string().contains("already taken"));
        fs::remove_file(registry::directory().unwrap().join(name)).unwrap();
    }
}
