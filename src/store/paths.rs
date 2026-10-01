use camino::{Utf8Path, Utf8PathBuf};

pub(crate) const NILES_DIR: &str = ".niles";
pub(crate) const WORKERS_DIR: &str = "worker";
pub(crate) const ARCHIVE_DIR: &str = "archive";

pub(crate) fn workers_dir(workspace: &Utf8Path) -> Utf8PathBuf {
    workspace.join(NILES_DIR).join(WORKERS_DIR)
}

pub(crate) fn archive_dir(workspace: &Utf8Path) -> Utf8PathBuf {
    workers_dir(workspace).join(ARCHIVE_DIR)
}
