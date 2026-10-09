use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};

use crate::util::{dated_directories, parse_timestamp_id};

use super::paths::archive_dir;

pub(crate) fn latest_worker_archive(
    workspace: &Utf8Path,
    worker: &str,
) -> Result<Option<Utf8PathBuf>> {
    Ok(worker_archives(workspace)?
        .into_iter()
        .filter(|(_, path)| {
            path.file_name()
                .and_then(|name| name.rsplit_once('-'))
                .is_some_and(|(id, _)| id == worker)
        })
        .max()
        .map(|(_, path)| path))
}

/// Every archived worker directory, with the time it was archived.
pub(crate) fn worker_archives(workspace: &Utf8Path) -> Result<Vec<(DateTime<Utc>, Utf8PathBuf)>> {
    dated_directories(&archive_dir(workspace), |name| {
        parse_timestamp_id(name.rsplit_once('-')?.1)
    })
}
