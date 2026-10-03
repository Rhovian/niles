use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Duration, Utc};

use super::registry;
use crate::{
    session, store,
    util::{parse_timestamp_id, read_dir_utf8_paths},
};

pub fn run(older_than: u32, apply: bool) -> Result<()> {
    let cutoff = Utc::now()
        .checked_sub_signed(Duration::days(i64::from(older_than)))
        .context("--older-than exceeds the timestamp range")?;
    for entry in registry::entries()? {
        if !entry.path.is_dir() {
            let link = registry::directory()?.join(entry.name.as_str());
            if apply {
                fs::remove_file(&link).with_context(|| format!("failed to remove {link}"))?;
            }
            print_removal(&link, apply);
            continue;
        }
        let archives = dated_directories(&store::archive_dir(&entry.path), |name| {
            parse_timestamp_id(name.rsplit_once('-')?.1)
        })?;
        let sessions = dated_directories(&session::sessions_dir(&entry.path), parse_timestamp_id)?;
        let newest = sessions.iter().map(|(timestamp, _)| *timestamp).max();
        let old_sessions = sessions
            .into_iter()
            .filter(|(timestamp, _)| Some(*timestamp) != newest);
        for (timestamp, path) in archives.into_iter().chain(old_sessions) {
            if timestamp < cutoff {
                if apply {
                    fs::remove_dir_all(&path)
                        .with_context(|| format!("failed to remove {path}"))?;
                }
                print_removal(&path, apply);
            }
        }
    }
    Ok(())
}

fn dated_directories(
    dir: &Utf8Path,
    parse: fn(&str) -> Option<DateTime<Utc>>,
) -> Result<Vec<(DateTime<Utc>, Utf8PathBuf)>> {
    let mut directories = Vec::new();
    for path in read_dir_utf8_paths(dir)? {
        let name = path.file_name().context("metadata entry has no name")?;
        if let Some(timestamp) = parse(name) {
            let metadata =
                fs::symlink_metadata(&path).with_context(|| format!("failed to inspect {path}"))?;
            if metadata.is_dir() {
                directories.push((timestamp, path));
            }
        }
    }
    Ok(directories)
}

fn print_removal(path: &Utf8Path, apply: bool) {
    let action = if apply { "removed" } else { "would remove" };
    println!("{action} {path}");
}
