//! Workspace wake delivery. Each poll prints at most one actionable line per live worker.
//! The workspace lock serializes presence replacement with delivery; worker cursor locks
//! serialize delivery with `send`. Output is flushed before its cursor is advanced.

use crate::{
    duration::parse_duration,
    wake::{self, WakeKind},
    worker,
};
use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use std::{
    fs,
    io::{ErrorKind, Read, Seek, SeekFrom},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    time::Duration,
};
pub(crate) mod cursor;
mod exit;
mod workspace;
use cursor::{cursor_path, open_lock_file, read_cursor, write_cursor};
pub(crate) use exit::WaitExit;
pub(crate) use workspace::wait;

pub const EXIT_WAKE: u8 = 0;
pub const EXIT_SUPERSEDED: u8 = 11;
pub const EXIT_TIMEOUT: u8 = 22;
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(2);
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const MAX_INTERVAL: Duration = Duration::from_secs(60 * 60);
// Includes the newline added at delivery.
const MAX_LINE_BYTES: usize = 4096;

/// Returns pre-send wakes to the sender while advancing the cursor under its lock.
pub(crate) fn advance_cursor(worker_id: &str) -> Result<Vec<String>> {
    Target::resolve(worker_id.to_owned())?.skip_to_end()
}

struct Target {
    id: String,
    dir: Utf8PathBuf,
    status: Utf8PathBuf,
    scanned: u64,
    identity: Option<(u64, u64)>,
}
impl Target {
    fn resolve(id: String) -> Result<Self> {
        let dir = worker::worker_dir(&id)?;
        let status = wake::status_log_path(&dir);
        Ok(Self {
            id,
            dir,
            status,
            scanned: 0,
            identity: None,
        })
    }
    /// Reads only the bytes appended since the last scan and returns the first actionable line
    /// with the offset just past it. A trailing partial line is left for the next poll.
    fn next_actionable(&mut self, persisted: u64) -> Result<Option<(WakeKind, String, u64)>> {
        let mut file = match fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&self.status)
        {
            Ok(file) => file,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", self.status));
            }
        };

        let metadata = file
            .metadata()
            .with_context(|| format!("failed to inspect {}", self.status))?;
        let identity = (metadata.dev(), metadata.ino());
        // A replacement invalidates the cache, not a cursor that send already advanced.
        if self.identity.is_some_and(|previous| previous != identity) {
            self.scanned = persisted;
        }
        self.identity = Some(identity);
        let len = metadata.len();
        // Truncation also invalidates the remembered offset in the same inode.
        if len < self.scanned {
            self.scanned = 0;
        }
        if len == self.scanned {
            return Ok(None);
        }

        file.seek(SeekFrom::Start(self.scanned))
            .with_context(|| format!("failed to seek {}", self.status))?;
        let mut appended = Vec::new();
        file.take(len - self.scanned)
            .read_to_end(&mut appended)
            .with_context(|| format!("failed to read {}", self.status))?;

        let mut complete = 0;
        for (end, line) in wake::complete_lines(&appended) {
            complete = end;
            if let Some(kind) = WakeKind::actionable(&line) {
                return Ok(Some((kind, render(&line), self.scanned + end as u64)));
            }
        }

        self.scanned += complete as u64;
        Ok(None)
    }

    fn next_report(&mut self, persisted: u64) -> Result<Option<(String, u64)>> {
        self.scanned = self.scanned.max(persisted);
        while let Some((kind, line, end)) = self.next_actionable(persisted)? {
            if kind != WakeKind::Closed {
                return Ok(Some((line, end)));
            }
            self.scanned = end;
        }
        Ok(None)
    }

    /// Consumes every actionable line currently in the log and persists the end position.
    fn skip_to_end(&mut self) -> Result<Vec<String>> {
        let path = cursor_path(&self.dir);
        let mut cursor = open_lock_file(&path)?;
        cursor
            .lock()
            .with_context(|| format!("failed to lock {path}"))?;
        let persisted = read_cursor(&mut cursor, &path)?;
        self.scanned = self.scanned.max(persisted);

        let mut skipped = Vec::new();
        while let Some((_, line, end)) = self.next_actionable(persisted)? {
            skipped.push(line);
            self.scanned = end;
        }
        // A failed scan leaves `scanned` just past the last complete line, which is where the
        // next wait should resume from.
        write_cursor(&mut cursor, &path, self.scanned)?;
        drop(cursor);
        Ok(skipped)
    }
}

/// Escapes control characters so a status line cannot rewrite the manager's terminal, and caps
/// the length so it cannot flood it.
fn render(line: &str) -> String {
    let mut rendered = String::with_capacity(line.len());
    for character in line.chars() {
        if character.is_control() {
            rendered.extend(character.escape_debug());
        } else {
            rendered.push(character);
        }
    }

    if rendered.len() < MAX_LINE_BYTES {
        return rendered;
    }
    let mut cut = MAX_LINE_BYTES - 1 - "... (truncated)".len();
    while cut > 0 && !rendered.is_char_boundary(cut) {
        cut -= 1;
    }
    rendered.truncate(cut);
    rendered.push_str("... (truncated)");
    rendered
}

pub(crate) fn parse_interval(value: &str) -> Result<Duration, String> {
    let interval = parse_duration(value).map_err(|error| error.to_string())?;
    if interval.is_zero() || interval > MAX_INTERVAL {
        return Err("wait `--interval` must be greater than zero and at most 1h".to_owned());
    }
    Ok(interval)
}

pub(crate) fn parse_timeout(value: &str) -> Result<Duration, String> {
    parse_duration(value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interval_is_positive_and_at_most_one_hour() {
        assert_eq!(parse_interval("1ms").unwrap(), Duration::from_millis(1));
        assert_eq!(parse_interval("1h").unwrap(), MAX_INTERVAL);
        for value in ["0", "3600001ms"] {
            assert!(parse_interval(value).unwrap_err().contains("--interval"));
        }
    }
    #[test]
    fn timeout_accepts_zero_and_has_no_policy_maximum() {
        assert_eq!(parse_timeout("0").unwrap(), Duration::ZERO);
        assert_eq!(
            parse_timeout("10000h").unwrap(),
            Duration::from_secs(36_000_000)
        );
    }
}
