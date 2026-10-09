use std::{fs, process};

use anyhow::{Context, Result};
use camino::Utf8Path;

use super::{Buckets, Usage};
use crate::store;

const CACHE_FILE: &str = "usage.json";

/// A closed session's buckets, from its `usage.json`, or read once with `read` and stored there.
///
/// A session whose transcript is gone stores empty buckets, so it is not searched for again.
pub(crate) fn closed_buckets(
    dir: &Utf8Path,
    read: impl FnOnce() -> Result<Option<Usage>>,
) -> Result<Buckets> {
    let path = dir.join(CACHE_FILE);
    if let Some(buckets) = store::read_optional_json(&path)? {
        return Ok(buckets);
    }
    let buckets = match read()? {
        Some(usage) => usage.buckets,
        None => Buckets::default(),
    };
    // race accepted: concurrent renders compute the same buckets and the last identical rename
    // wins. The staging name is per writer, so one writer never truncates a file another is about
    // to rename.
    let staging = dir.join(format!("{CACHE_FILE}.{}.tmp", process::id()));
    let body = serde_json::to_string(&buckets).context("failed to serialize usage buckets")?;
    fs::write(&staging, body).with_context(|| format!("failed to write {staging}"))?;
    fs::rename(&staging, &path).with_context(|| format!("failed to replace {path}"))?;
    Ok(buckets)
}
