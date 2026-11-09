use anyhow::{Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use sha2::{Digest, Sha256};

pub(crate) const WORKERS_DIR: &str = "worker";
pub(crate) const ARCHIVE_DIR: &str = "archive";

pub(crate) fn project_dir(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    if !workspace.is_absolute() {
        bail!("workspace path is not absolute: {workspace}");
    }
    let home = crate::util::home_dir()?;
    if !home.is_absolute() {
        bail!("HOME must be absolute: {home}");
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
            for other in &dirs[index + 1..] {
                assert_ne!(dir.as_str().to_lowercase(), other.as_str().to_lowercase());
                assert!(!dir.starts_with(other) && !other.starts_with(dir));
            }
        }
    }

    #[test]
    fn long_keys_use_bounded_components_and_disjoint_state_leaves() {
        let root = crate::util::home_dir().unwrap().join(".niles/state");
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
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::remove_dir_all(&dir).unwrap();
        }
        assert!(project_dir(Utf8Path::new("relative/path")).is_err());
    }
}
