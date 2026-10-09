use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};

use crate::util::{dated_directories, parse_timestamp_id, read_dir_utf8_paths};

use super::paths::archive_dir;

pub(crate) fn latest_worker_archive(
    workspace: &Utf8Path,
    worker: &str,
) -> Result<Option<Utf8PathBuf>> {
    Ok(read_dir_utf8_paths(&archive_dir(workspace))?
        .into_iter()
        .filter(|path| path.is_dir())
        .filter_map(|path| {
            let archived_at = worker_archive_timestamp(worker, path.file_name()?)?;
            Some((archived_at, path))
        })
        .max_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)))
        .map(|(_, path)| path))
}

fn worker_archive_timestamp(worker: &str, archive_name: &str) -> Option<DateTime<Utc>> {
    let timestamp = archive_name.strip_prefix(&format!("{worker}-"))?;
    parse_timestamp_id(timestamp)
}

/// Every archived worker directory, with the time it was archived.
pub(crate) fn worker_archives(workspace: &Utf8Path) -> Result<Vec<(DateTime<Utc>, Utf8PathBuf)>> {
    dated_directories(&archive_dir(workspace), |name| {
        parse_timestamp_id(name.rsplit_once('-')?.1)
    })
}
