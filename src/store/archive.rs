use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, NaiveDateTime, Utc};

use crate::util::read_dir_utf8_paths;

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

#[expect(
    clippy::disallowed_methods,
    reason = "timestamp parse failure can be a prefix-sibling worker archive, so archive discovery must skip it"
)]
fn worker_archive_timestamp(worker: &str, archive_name: &str) -> Option<DateTime<Utc>> {
    let timestamp = archive_name.strip_prefix(&format!("{worker}-"))?;
    let timestamp = NaiveDateTime::parse_from_str(timestamp, "%Y%m%dT%H%M%S%fZ").ok()?;
    Some(DateTime::from_naive_utc_and_offset(timestamp, Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::timestamp_id;
    use chrono::Utc;

    /// The archive directory name is written by `util::timestamp_id` and read back here; this
    /// round-trips them so a format drift in either place cannot silently break archive discovery
    /// (which fails quietly and makes every archived report unreachable).
    #[test]
    fn archive_name_round_trips_through_encoder_and_parser() {
        let archived_at = DateTime::<Utc>::from_timestamp(1_758_341_218, 290_814_000).unwrap();
        let name = format!("auth-fix-{}", timestamp_id(&archived_at));
        let parsed =
            worker_archive_timestamp("auth-fix", &name).expect("archive name should parse");
        assert_eq!(parsed, archived_at);
    }
}
