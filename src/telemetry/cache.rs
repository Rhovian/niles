use anyhow::Result;
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
    store::write_json(&path, &buckets)?;
    Ok(buckets)
}
