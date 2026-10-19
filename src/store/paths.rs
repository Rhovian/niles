use std::env;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

pub(crate) const WORKERS_DIR: &str = "worker";
pub(crate) const ARCHIVE_DIR: &str = "archive";

const MAX_KEY_BYTES: usize = 200;
const KEY_PREFIX_CHARS: usize = 40;

pub(crate) fn project_dir(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    let destination = project_path(workspace)?;
    super::migrate::legacy(&workspace.join(".niles"), &destination)?;
    Ok(destination)
}

pub(crate) fn project_path(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    if !workspace.is_absolute() {
        bail!("workspace path is not absolute: {workspace}");
    }
    // camino 1.2.6 (src/lib.rs, Utf8Path) supplies path components, but no reversible single-component encoding.
    // Escape '%' and '-' as well as separators so distinct absolute paths cannot collide.
    let key = workspace
        .as_str()
        .replace('%', "%25")
        .replace('-', "%2D")
        .replace('/', "-");
    let key = if key.len() > MAX_KEY_BYTES {
        // uuid 1.26.1 src/v5.rs provides stable name hashing; bound the readable prefix
        // by characters so Unicode stays intact and the component remains below NAME_MAX.
        let prefix: String = key.chars().take(KEY_PREFIX_CHARS).collect();
        format!(
            "{prefix}--{}",
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, workspace.as_str().as_bytes())
        )
    } else {
        key
    };
    let home = Utf8PathBuf::from(env::var("HOME").context("HOME is missing")?);
    if !home.is_absolute() {
        bail!("HOME must be absolute: {home}");
    }
    Ok(home.join(".niles/state").join(key))
}

pub(crate) fn workers_dir(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    Ok(project_dir(workspace)?.join(WORKERS_DIR))
}

pub(crate) fn archive_dir(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    Ok(workers_dir(workspace)?.join(ARCHIVE_DIR))
}
