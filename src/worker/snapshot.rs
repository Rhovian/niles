use std::{fs, io::ErrorKind};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    store,
    wake::{self, WakeKind},
};

use super::meta::{WorkerMeta, meta_path, read_meta_if_exists};

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

    /// The bytes the given cursor has not consumed yet. A cursor that cannot slice this log — past
    /// its end, or mid-character in one that has been rewritten — describes a log `wait` will
    /// rescan from the start, so nothing is delivered.
    pub(super) fn undelivered(&self, delivered: usize) -> Option<&[u8]> {
        let log = self.log.as_deref()?;
        Some(match log.get(delivered..) {
            Some(undelivered) => undelivered,
            None => log,
        })
    }
}

/// The workspace's live workers: those with a `meta.json`, which is what makes a directory a
/// worker rather than a leftover.
pub(crate) fn worker_snapshot(workspace: &Utf8Path) -> Result<Vec<WorkerSnapshot>> {
    let mut workers = Vec::new();
    for entry in store::resolve_worker_locations_in(workspace)? {
        if !meta_path(&entry.worker_dir).exists() {
            continue;
        }
        let (meta, read_error) = match read_meta_if_exists(&entry.worker_dir) {
            Ok(Some(meta)) => (Some(meta), None),
            Ok(None) => continue,
            Err(err) => (None, Some(format!("{err:#}"))),
        };
        let log = status_log(&entry.worker_dir)?;
        let mut snapshot = WorkerSnapshot::new(entry.id, entry.worker_dir, log);
        snapshot.meta = meta;
        snapshot.read_error = read_error;
        workers.push(snapshot);
    }
    workers.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(workers)
}

/// How long a worker's status log is, in bytes, without reading it twice.
pub(crate) fn status_log_len(worker_dir: &Utf8Path) -> Result<u64> {
    Ok(status_log(worker_dir)?.map_or(0, |log| log.len() as u64))
}

fn status_log(worker_dir: &Utf8Path) -> Result<Option<Vec<u8>>> {
    let status_path = wake::status_log_path(worker_dir);
    match fs::read(&status_path) {
        Ok(body) => Ok(Some(body)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed to read {status_path}")),
    }
}

/// The last actionable line in the log, complete lines only.
///
/// A trailing partial line is still being written — `niles wait` leaves it for the next poll, and
/// so does this: nudging a report before the newline that finishes it would have the lead look at
/// a line the worker has not finished saying.
fn last_actionable_wake(log: &[u8]) -> Option<ActionableWake> {
    let mut end = 0u64;
    let mut found = None;
    for raw in log.split_inclusive(|byte| *byte == b'\n') {
        if !raw.ends_with(b"\n") {
            break;
        }
        end += raw.len() as u64;
        let line = String::from_utf8_lossy(raw);
        let line = line.trim_end_matches('\n').trim_end_matches('\r');
        if wake::is_actionable_wake(line)
            && let Some(kind) = WakeKind::parse_line(line)
        {
            found = Some(ActionableWake { end, kind });
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
        assert_eq!(
            WorkerSnapshot::new("impl".to_owned(), Utf8PathBuf::from("/w"), None).log_len,
            0
        );
    }

    #[test]
    fn the_last_actionable_line_carries_its_end_offset() {
        let log = "working: launch\nworking: busy\ndone: shipped\n";
        let wake = snapshot("impl", log).last_actionable.unwrap();

        assert_eq!(wake.kind, WakeKind::Done);
        assert_eq!(wake.end, log.len() as u64);
    }

    /// `done: x` → follow-up → `done: y`: the last one is what the log ends on, and its offset is
    /// past the first, which is how a second report is told from a replay of the first.
    #[test]
    fn only_the_last_actionable_line_is_kept() {
        let wake = snapshot("impl", "done: one\nworking: more\ndone: two\n")
            .last_actionable
            .unwrap();

        assert_eq!(wake.kind, WakeKind::Done);
        assert_eq!(wake.end, 34);
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
    fn a_worker_with_no_log_has_nothing_to_report() {
        let worker = WorkerSnapshot::new("impl".to_owned(), Utf8PathBuf::from("/w"), None);

        assert_eq!(worker.log_len, 0);
        assert_eq!(worker.last_actionable, None);
        assert_eq!(worker.last_status_line(), None);
    }

    #[test]
    fn the_last_status_line_ignores_trailing_blanks() {
        let worker = snapshot("impl", "working: launch\ndone: shipped\n\n");

        assert_eq!(worker.last_status_line().as_deref(), Some("done: shipped"));
    }

    #[test]
    fn undelivered_slices_from_the_cursor_and_falls_back_to_the_whole_log() {
        let worker = snapshot("impl", "done: one\ndone: two\n");

        assert_eq!(worker.undelivered(10), Some(&b"done: two\n"[..]));
        assert_eq!(worker.undelivered(0), Some(&b"done: one\ndone: two\n"[..]));
        assert_eq!(
            worker.undelivered(200),
            Some(&b"done: one\ndone: two\n"[..])
        );
    }
}
