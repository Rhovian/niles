use std::collections::BTreeMap;

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::util::read_dir_utf8_paths;

use super::{
    global::{global_worker_archives, write_global_worker_archive_pointer},
    paths::current_workers_dir,
};

pub(crate) fn register_worker_archive(
    worker: &str,
    archive_dir: &Utf8Path,
    archived_at: DateTime<Utc>,
) -> Result<WorkerArchivePointer> {
    let pointer = WorkerArchivePointer {
        id: worker.to_owned(),
        archive_dir: archive_dir.to_path_buf(),
        archived_at,
    };
    write_global_worker_archive_pointer(&pointer)?;
    Ok(pointer)
}

pub(crate) fn resolve_worker_archives(worker: &str) -> Result<Vec<WorkerArchivePointer>> {
    let mut archives = BTreeMap::new();
    for archive in local_worker_archives(worker)? {
        archives.insert(archive.archive_dir.clone(), archive);
    }
    for archive in global_worker_archives(worker)? {
        archives.insert(archive.archive_dir.clone(), archive);
    }

    let mut archives = archives.into_values().collect::<Vec<_>>();
    archives.sort_by(|left, right| {
        left.archived_at
            .cmp(&right.archived_at)
            .then_with(|| left.archive_dir.cmp(&right.archive_dir))
    });
    Ok(archives)
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct WorkerArchivePointer {
    pub(crate) id: String,
    pub(crate) archive_dir: Utf8PathBuf,
    pub(crate) archived_at: DateTime<Utc>,
}

fn worker_archive_root(workers_dir: &Utf8Path) -> Utf8PathBuf {
    workers_dir.join("archive")
}

fn local_worker_archives(worker: &str) -> Result<Vec<WorkerArchivePointer>> {
    let archive_root = worker_archive_root(&current_workers_dir()?);
    let mut archives = Vec::new();
    for path in read_dir_utf8_paths(&archive_root)? {
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        let Some(archived_at) = worker_archive_timestamp(worker, name) else {
            continue;
        };
        archives.push(WorkerArchivePointer {
            id: worker.to_owned(),
            archive_dir: path,
            archived_at,
        });
    }
    Ok(archives)
}

#[expect(
    clippy::disallowed_methods,
    reason = "timestamp parse failure can be a prefix-sibling worker archive, so archive discovery must skip it"
)]
fn worker_archive_timestamp(worker: &str, archive_name: &str) -> Option<DateTime<Utc>> {
    let timestamp = archive_name.strip_prefix(&format!("{worker}-"))?;
    let timestamp = NaiveDateTime::parse_from_str(timestamp, "%Y%m%dT%H%M%S%fZ").ok()?;
    Some(DateTime::from_naive_utc_and_offset(timestamp, Utc))
}
