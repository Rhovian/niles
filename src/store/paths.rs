use std::fmt::Write;

use anyhow::{Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

pub(crate) const WORKERS_DIR: &str = "worker";
pub(crate) const ARCHIVE_DIR: &str = "archive";
// Two hex digits per byte keep each component below the usual 255-byte name limit.
const KEY_CHUNK_BYTES: usize = 120;

pub(crate) fn project_dir(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    if !workspace.is_absolute() {
        bail!("workspace path is not absolute: {workspace}");
    }
    let home = crate::util::home_dir()?;
    if !home.is_absolute() {
        bail!("HOME must be absolute: {home}");
    }
    let mut dir = home.join(".niles/state/v2");
    // Encode bytes rather than path components to preserve case and Unicode distinctions.
    // Camino exposes UTF-8, but has no filesystem key encoding:
    // https://docs.rs/camino/1.2.6/camino/struct.Utf8Path.html#method.as_str
    for chunk in workspace.as_str().as_bytes().chunks(KEY_CHUNK_BYTES) {
        let mut encoded = String::with_capacity(chunk.len() * 2);
        for byte in chunk {
            write!(encoded, "{byte:02x}")?;
        }
        dir.push(encoded);
    }
    // Outside the hex alphabet, so no longer key can descend into this project's state.
    Ok(dir.join("@state"))
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
    fn keys_encode_workspace_bytes() {
        let root = crate::util::home_dir().unwrap().join(".niles/state/v2");
        for (workspace, key) in [
            ("/", "2f"),
            ("/code/my-app", "2f636f64652f6d792d617070"),
            ("/code/my/app", "2f636f64652f6d792f617070"),
            ("/é", "2fc3a9"),
        ] {
            assert_eq!(
                project_dir(Utf8Path::new(workspace)).unwrap(),
                root.join(key).join("@state")
            );
        }
        assert!(project_dir(Utf8Path::new("relative/path")).is_err());
    }

    #[test]
    fn keys_separate_separators_case_and_reserved_names() {
        let workspaces = ["/a-b", "/a/b", "/A-b", "/a/@state", "/a", "/a/v2"];
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
        let workspace = format!("/{}", "a".repeat(239));
        let parent = project_dir(Utf8Path::new(&workspace)).unwrap();
        let child = project_dir(Utf8Path::new(&format!("{workspace}/é"))).unwrap();
        assert!(!child.starts_with(&parent));
        assert!(!parent.starts_with(&child));
        let root = crate::util::home_dir().unwrap().join(".niles/state/v2");
        for dir in [parent, child] {
            assert!(dir.starts_with(&root), "key must use the v2 namespace");
            let key = dir.strip_prefix(&root).unwrap();
            assert!(key.iter().all(|component| component.len() <= 240));
            assert_eq!(key.file_name(), Some("@state"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }
}
