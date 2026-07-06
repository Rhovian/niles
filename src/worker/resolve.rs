use anyhow::{Context, Result};
use camino::Utf8PathBuf;

use crate::{
    store::{self, WorkerLocation},
    wake,
};

use super::{
    meta::{metadata_status_path, read_meta_if_exists},
    validation::validate_id,
};

pub fn status_log_path(id: &str) -> Result<Utf8PathBuf> {
    validate_id(id)?;
    let location = resolve_worker(id)?;
    if let Some(meta) = read_meta_if_exists(&location.worker_dir)? {
        return Ok(metadata_status_path(&meta, &location.worker_dir));
    }
    Ok(wake::status_log_path(&location.worker_dir))
}

pub(super) fn resolve_worker(id: &str) -> Result<WorkerLocation> {
    resolve_worker_if_exists(id)?.with_context(|| format!("unknown worker id '{id}'"))
}

pub(super) fn resolve_worker_if_exists(id: &str) -> Result<Option<WorkerLocation>> {
    validate_id(id)?;
    store::resolve_worker_location(id)
}

pub(super) fn resolve_live_worker_if_exists(id: &str) -> Result<Option<WorkerLocation>> {
    let Some(location) = resolve_worker_if_exists(id)? else {
        return Ok(None);
    };
    Ok(read_meta_if_exists(&location.worker_dir)?
        .is_some()
        .then_some(location))
}

pub(super) fn no_live_worker_message(id: &str) -> String {
    match latest_archive(id) {
        Ok(Some(archive)) => format!(
            "no live worker '{id}'; latest archive: {}",
            archive.archive_dir
        ),
        Ok(None) => format!("no live worker '{id}'"),
        Err(err) => format!("no live worker '{id}'; failed to inspect archives: {err}"),
    }
}

pub(super) fn latest_archive(id: &str) -> Result<Option<store::WorkerArchivePointer>> {
    Ok(store::resolve_worker_archives(id)?
        .into_iter()
        .rev()
        .find(|archive| archive.archive_dir.exists()))
}
