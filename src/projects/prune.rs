use std::fs;

use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{Duration, Utc};

use super::registry;
use crate::{session, store};

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
        let archives = store::worker_archives(&entry.path)?;
        let sessions = session::session_dirs(&entry.path)?;
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

fn print_removal(path: &Utf8Path, apply: bool) {
    let action = if apply { "removed" } else { "would remove" };
    println!("{action} {path}");
}
