use std::{fs, io::ErrorKind};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, TimeDelta, Utc};

use crate::{
    store,
    wait::cursor::{cursor_path, parse_cursor, wait_running},
    wake::{self, WakeKind},
};

use super::meta::{WorkerMeta, read_meta_if_exists};

/// The last actionable line in a worker's status log, with the byte offset just past it.
///
/// The offset is what keys both questions asked of a log: whether it holds something the lead has
/// not been told about, and whether it answers a check-in armed at some earlier length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActionableWake {
    pub(crate) end: u64,
    pub(crate) kind: WakeKind,
}

/// One live worker, read once and read-only.
///
/// `niles workers` renders from this and the watcher decides from it, so neither keeps its own
/// idea of which workers are live or of where a line begins in a status log.
pub(crate) struct WorkerSnapshot {
    pub(crate) id: String,
    pub(crate) worker_dir: Utf8PathBuf,
    /// `None` when `meta.json` is missing or unreadable; `read_error` then says which of the two.
    pub(super) meta: Option<WorkerMeta>,
    pub(super) read_error: Option<String>,
    /// The status log's bytes, or `None` while there is no log to read.
    pub(super) log: Option<Vec<u8>>,
    /// Byte length of `log`. Report detection keys on the length rather than the state word, so
    /// `done:` → follow-up → `done:` counts as two reports.
    pub(crate) log_len: u64,
    pub(crate) last_actionable: Option<ActionableWake>,
    /// The wake cursor: how far `niles wait` has delivered. No cursor means no wait has ever
    /// consumed a line, which is position zero.
    cursor: u64,
    /// Whether a `niles wait` on this worker was running when the snapshot was taken.
    pub(crate) wait_running: bool,
}

impl WorkerSnapshot {
    /// Snapshots a worker's status log. The display fields are the reader's to fill.
    pub(crate) fn new(id: String, worker_dir: Utf8PathBuf, log: Option<Vec<u8>>) -> Self {
        let log_len = log.as_ref().map_or(0, |bytes| bytes.len() as u64);
        let last_actionable = log.as_deref().and_then(last_actionable_wake);
        Self {
            id,
            worker_dir,
            meta: None,
            read_error: None,
            log,
            log_len,
            last_actionable,
            cursor: 0,
            wait_running: false,
        }
    }

    /// The last non-empty status line, as `niles workers` renders it.
    pub(super) fn last_status_line(&self) -> Option<String> {
        String::from_utf8_lossy(self.log.as_deref()?)
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(str::to_owned)
    }

    /// How far into this log `niles wait` has delivered. A cursor past the end describes a log that
    /// has since been replaced, which `wait` rescans from the start, so it has delivered none of it.
    pub(crate) fn delivered(&self) -> u64 {
        if self.cursor <= self.log_len {
            self.cursor
        } else {
            0
        }
    }

    /// The bytes `niles wait` has not delivered yet.
    pub(super) fn undelivered(&self) -> Option<&[u8]> {
        let log = self.log.as_deref()?;
        // `delivered` is at most `log_len`, which is `log.len()`.
        Some(&log[self.delivered() as usize..])
    }

    /// The recorded pane and workspace while an otherwise-silent worker is still starting.
    pub(crate) fn startup_target(
        &self,
        now: DateTime<Utc>,
        window: TimeDelta,
    ) -> Option<(&str, &Utf8Path)> {
        if self.log_len != 0 {
            return None;
        }
        let meta = self.meta.as_ref()?;
        let created_at = meta.created_at;
        let age = now.signed_duration_since(created_at);
        (age >= TimeDelta::zero() && age <= window).then_some((&meta.window, meta.agent_dir()))
    }
}

/// The workspace's live workers: those with a `meta.json`, which is what makes a directory a
/// worker rather than a leftover.
pub(crate) fn worker_snapshot(workspace: &Utf8Path) -> Result<Vec<WorkerSnapshot>> {
    let mut workers = Vec::new();
    for entry in store::worker_locations(workspace)? {
        let (meta, read_error) = match read_meta_if_exists(&entry.worker_dir) {
            Ok(Some(meta)) => (Some(meta), None),
            Ok(None) => continue,
            Err(err) => (None, Some(format!("{err:#}"))),
        };
        let log = status_log(&entry.worker_dir)?;
        // Probed before the cursor is read: a wait that finishes in between has advanced the
        // cursor before it stopped running, so a line read above is either being delivered or
        // shows as delivered.
        let wait_running = wait_running(&entry.worker_dir)?;
        let cursor = read_wake_cursor(&entry.worker_dir)?;
        let mut snapshot = WorkerSnapshot::new(entry.id, entry.worker_dir, log);
        snapshot.meta = meta;
        snapshot.read_error = read_error;
        snapshot.cursor = cursor;
        snapshot.wait_running = wait_running;
        workers.push(snapshot);
    }
    Ok(workers)
}

fn read_wake_cursor(worker_dir: &Utf8Path) -> Result<u64> {
    let path = cursor_path(worker_dir);
    let Some(body) = crate::util::read_optional_string(&path)? else {
        return Ok(0);
    };
    parse_cursor(&body, &path)
}

/// How long a worker's status log is, in bytes, without reading it twice.
pub(crate) fn status_log_len(worker_dir: &Utf8Path) -> Result<u64> {
    Ok(status_log(worker_dir)?.map_or(0, |log| log.len() as u64))
}

pub(super) fn status_log(worker_dir: &Utf8Path) -> Result<Option<Vec<u8>>> {
    let status_path = wake::status_log_path(worker_dir);
    match fs::read(&status_path) {
        Ok(body) => Ok(Some(body)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed to read {status_path}")),
    }
}

/// The last actionable line in the log, complete lines only.
///
/// A trailing partial line is still being written, so it stays for the next poll.
fn last_actionable_wake(log: &[u8]) -> Option<ActionableWake> {
    let mut found = None;
    for (end, line) in wake::complete_lines(log) {
        if let Some(kind) = WakeKind::actionable(&line) {
            found = Some(ActionableWake {
                end: end as u64,
                kind,
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(id: &str, log: &str) -> WorkerSnapshot {
        WorkerSnapshot::new(
            id.to_owned(),
            Utf8PathBuf::from("/w"),
            Some(log.as_bytes().to_vec()),
        )
    }

    #[test]
    fn log_length_is_the_byte_length() {
        assert_eq!(snapshot("impl", "done: x\n").log_len, 8);
        assert_eq!(snapshot("impl", "working: café\n").log_len, 15);
        let empty = WorkerSnapshot::new("impl".to_owned(), Utf8PathBuf::from("/w"), None);
        assert_eq!(empty.log_len, 0);
        assert_eq!(empty.last_actionable, None);
        assert_eq!(empty.last_status_line(), None);
    }

    #[test]
    fn the_last_actionable_line_carries_its_end_offset() {
        let log = "done: one\nworking: more\ndone: shipped\n";
        let wake = snapshot("impl", log).last_actionable.unwrap();

        assert_eq!(wake.kind, WakeKind::Done);
        assert_eq!(wake.end, log.len() as u64);
        assert!(wake.end > "done: one\n".len() as u64);
    }

    #[test]
    fn actionable_lines_tolerate_carriage_returns() {
        let wake = snapshot("impl", "done: shipped\r\n")
            .last_actionable
            .unwrap();

        assert_eq!(wake.kind, WakeKind::Done);
    }

    /// The newline is what finishes a line: without it, the worker is still saying it.
    #[test]
    fn a_trailing_partial_line_is_not_actionable_yet() {
        assert_eq!(snapshot("impl", "done: shipped").last_actionable, None);
        assert!(
            snapshot("impl", "done: shipped\n")
                .last_actionable
                .is_some()
        );
    }

    #[test]
    fn working_lines_are_not_actionable() {
        assert_eq!(snapshot("impl", "working: launch\n").last_actionable, None);
        assert_eq!(snapshot("impl", "note: launch\n").last_actionable, None);
    }

    #[test]
    fn the_last_status_line_ignores_trailing_blanks() {
        let worker = snapshot("impl", "working: launch\ndone: shipped\n\n");

        assert_eq!(worker.last_status_line().as_deref(), Some("done: shipped"));
    }

    #[test]
    fn undelivered_slices_from_the_cursor_and_falls_back_to_the_whole_log() {
        let mut worker = snapshot("impl", "done: one\ndone: two\n");

        worker.cursor = 10;
        assert_eq!(worker.undelivered(), Some(&b"done: two\n"[..]));
        worker.cursor = 0;
        assert_eq!(worker.undelivered(), Some(&b"done: one\ndone: two\n"[..]));
        worker.cursor = 200;
        assert_eq!(worker.delivered(), 0);
        assert_eq!(worker.undelivered(), Some(&b"done: one\ndone: two\n"[..]));
    }
}
