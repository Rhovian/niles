use crate::util;
use crate::wait::cursor;
use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use sha2::{Digest, Sha256};
use std::fs::{self, File, TryLockError};

pub(crate) const WORKERS_DIR: &str = "worker";
pub(crate) const ARCHIVE_DIR: &str = "archive";

pub(crate) fn project_dir(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    if !workspace.is_absolute() {
        bail!("workspace path is not absolute: {workspace}");
    }
    let home = util::home_dir()?;
    Ok(home
        .join(".niles/state")
        .join(format!(
            "{:x}",
            Sha256::digest(workspace.as_str().as_bytes())
        ))
        .join("@state"))
}

pub(crate) fn workers_dir(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    Ok(project_dir(workspace)?.join(WORKERS_DIR))
}

pub(crate) fn archive_dir(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    Ok(workers_dir(workspace)?.join(ARCHIVE_DIR))
}

#[derive(Debug, serde::Serialize)]
pub(crate) enum Scope<'a> {
    Project(&'a Utf8Path),
    #[serde(rename = "Project")] // Deregistration excludes shared project reservations.
    Deregister(&'a Utf8Path),
    Worker(&'a Utf8Path, &'a str),
    Tree(&'a Utf8Path),
    Pane(&'a str),
}

pub(crate) struct Lock {
    _file: File,
}
impl Scope<'_> {
    pub(crate) fn reserve(self) -> Result<Lock> {
        // Never unlink these global lock inodes, including during deregistration.
        let root = util::home_dir()?.join(".niles/locks");
        fs::create_dir_all(&root)?;
        let path = root.join(format!("{:x}", Sha256::digest(serde_json::to_vec(&self)?)));
        let file = cursor::open_lock_file(&path)?;
        // https://doc.rust-lang.org/1.89.0/std/fs/struct.File.html#method.try_lock
        match self {
            Self::Project(_) => file.try_lock_shared(),
            Self::Pane(_) => file.lock().map_err(TryLockError::Error),
            Self::Deregister(_) | Self::Worker(..) | Self::Tree(_) => file.try_lock(),
        }
        .with_context(|| format!("{self:?} is busy; cannot lock {path}"))?;
        Ok(Lock { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util;
    use std::fs;

    #[test]
    fn keys_separate_separators_case_and_reserved_names() {
        let workspaces = [
            "/a-b",
            "/a/b",
            "/A-b",
            "/a/@state",
            "/a",
            "/a/state",
            "/é",
            "/e",
        ];
        let dirs: Vec<_> = workspaces
            .iter()
            .map(|path| project_dir(Utf8Path::new(path)).unwrap())
            .collect();
        for (index, dir) in dirs.iter().enumerate() {
            for other in dirs.iter().skip(index).skip(1) {
                assert_ne!(dir.as_str().to_lowercase(), other.as_str().to_lowercase());
                assert!(!dir.starts_with(other) && !other.starts_with(dir));
            }
        }
    }

    #[test]
    fn long_keys_use_bounded_components_and_disjoint_state_leaves() {
        let root = util::home_dir().unwrap().join(".niles/state");
        for workspace in [
            "/".to_owned(),
            "/é".into(),
            format!("/{}", "a/".repeat(libc::PATH_MAX as usize / 3)),
        ] {
            let dir = project_dir(Utf8Path::new(&workspace)).unwrap();
            let key = dir.strip_prefix(&root).unwrap();
            assert_eq!(key.iter().count(), 2);
            assert_eq!(key.iter().next().unwrap().len(), 64);
            assert_eq!(key.file_name(), Some("@state"));
            fs::create_dir_all(&dir).unwrap();
            fs::remove_dir_all(&dir).unwrap();
        }
        assert!(project_dir(Utf8Path::new("relative/path")).is_err());
    }
}
