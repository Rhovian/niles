use anyhow::{Context, Result};
use camino::Utf8PathBuf;

use crate::{
    store,
    tmux::{self, TargetState, WindowTarget},
    wake,
};

use super::{
    meta::{WorkerMeta, read_meta_if_exists},
    validation::validate_id,
};

pub fn status_log_path(id: &str) -> Result<Utf8PathBuf> {
    validate_id(id)?;
    Ok(wake::status_log_path(&resolve_worker(id)?))
}

/// Whether this worker's tmux window is definitively gone, so nothing can ever append to its
/// status log again.
///
/// Only a confirmed absence counts. A transient tmux failure, an ambiguous legacy candidate, and
/// a window found alive under a new name all report `false`: a wait that stops early on a healthy
/// worker is a worse failure than one that waits out its timeout.
pub fn window_is_gone(id: &str) -> Result<bool> {
    validate_id(id)?;
    let Some(worker_dir) = resolve_worker_if_exists(id)? else {
        return Ok(false);
    };
    let Some(meta) = read_meta_if_exists(&worker_dir)? else {
        return Ok(false);
    };

    Ok(match window_state(&meta) {
        TargetState::PaneExited | TargetState::WindowDead | TargetState::OrphanGone => true,
        TargetState::Live
        | TargetState::OrphanRecovered { .. }
        | TargetState::OrphanLegacyCandidate { .. }
        | TargetState::Unknown { .. } => false,
    })
}

pub(super) fn window_state(meta: &WorkerMeta) -> TargetState {
    match WindowTarget::parse(&meta.window) {
        Ok(target) => tmux::target_state(&target, &meta.project, &meta.id),
        Err(err) => TargetState::Unknown {
            error: format!("{err:#}"),
        },
    }
}

pub(crate) fn resolve_worker(id: &str) -> Result<Utf8PathBuf> {
    resolve_worker_if_exists(id)?.with_context(|| format!("unknown worker id '{id}'"))
}

pub(super) fn resolve_worker_if_exists(id: &str) -> Result<Option<Utf8PathBuf>> {
    validate_id(id)?;
    store::resolve_worker_location(id)
}

pub(super) fn resolve_live_worker_if_exists(id: &str) -> Result<Option<Utf8PathBuf>> {
    let Some(worker_dir) = resolve_worker_if_exists(id)? else {
        return Ok(None);
    };
    Ok(read_meta_if_exists(&worker_dir)?
        .is_some()
        .then_some(worker_dir))
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

pub(super) fn latest_archive(id: &str) -> Result<Option<store::WorkerArchive>> {
    Ok(store::resolve_worker_archives(id)?
        .into_iter()
        .rev()
        .find(|archive| archive.archive_dir.exists()))
}
