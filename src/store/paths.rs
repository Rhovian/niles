use anyhow::{Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

pub(crate) const WORKERS_DIR: &str = "worker";
pub(crate) const ARCHIVE_DIR: &str = "archive";

pub(crate) fn project_dir(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    if !workspace.is_absolute() {
        bail!("workspace path is not absolute: {workspace}");
    }
    // Operator-chosen Claude-style key: slash/hyphen collisions are accepted.
    let key = workspace.as_str().replace('/', "-");
    let home = crate::util::home_dir()?;
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
