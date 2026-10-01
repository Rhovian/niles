//! Wake delivery.
//!
//! A worker appends status lines to `<worker-dir>/status.log`. `niles wait` blocks until one of
//! those lines is actionable, prints it, and records the byte offset just past it in
//! `<worker-dir>/status.cursor` so the next wait resumes after it. That cursor is the whole
//! coordination mechanism: each actionable line is delivered exactly once because delivery and
//! cursor advance happen together.
//!
//! Two concurrent waits on one worker are allowed. They are serialised by an exclusive `flock`
//! held across the read-scan-advance window, so exactly one of them is handed any given line and
//! the other simply keeps waiting. The lock is the whole mutual-exclusion story: the kernel
//! releases it when the holder dies, which is why this needs no pid, token, heartbeat, staleness
//! threshold, or takeover path to recover from a waiter that went away.

use std::{
    fs,
    io::{ErrorKind, Read, Seek, SeekFrom},
    os::unix::fs::OpenOptionsExt,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use camino::Utf8PathBuf;

use crate::{
    wake::{self, WakeKind},
    worker,
};

pub(crate) mod cursor;
mod exit;

use cursor::{cursor_path, open_cursor, read_cursor, write_cursor};
use exit::Outcome;
pub(crate) use exit::WaitExit;

pub const EXIT_WAKE: u8 = 0;
pub const EXIT_WORKER_CLOSED: u8 = 10;
pub const EXIT_TIMEOUT: u8 = 22;

/// Poll interval used by `niles wait` and by `niles send --wait`.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(2);
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// Largest accepted `--interval`. A poll interval beyond the default timeout is always a typo.
const MAX_INTERVAL_SECS: f64 = 3600.0;

/// Longest status line rendered to the manager. A worker can append a line of any length; the
/// manager's terminal should not have to absorb it.
const MAX_LINE_BYTES: usize = 4096;

/// Backstop only; each check costs a tmux subprocess per worker.
const WINDOW_CHECK_INTERVAL: Duration = Duration::from_secs(5);

pub(crate) enum WaitOn {
    Workers(Vec<String>),
    Task(String),
}

pub fn wait(on: WaitOn, interval: Duration, timeout: Duration) -> Result<WaitExit> {
    let mut targets = resolve_targets(on)?;
    let prefix_worker_id = targets.len() > 1;
    let subject = timeout_subject(&targets);

    let deadline = Instant::now() + timeout;
    loop {
        for target in &mut targets {
            if let Some(outcome) = target.poll()? {
                return Ok(WaitExit::from_outcome(outcome, prefix_worker_id));
            }
        }

        let now = Instant::now();
        if now >= deadline {
            return Ok(WaitExit::from_outcome(
                Outcome::Timeout { subject, timeout },
                prefix_worker_id,
            ));
        }
        thread::sleep(interval.min(deadline - now));
    }
}

/// Advances a worker's wake cursor past everything already in its status log, returning any
/// actionable lines it skipped.
///
/// `send` calls this so a line the worker wrote *before* the message cannot satisfy the wait that
/// follows it. The skipped lines are returned rather than dropped silently, because a `blocked:`
/// that lands just before a send is real information the operator should still see.
pub(crate) fn advance_cursor(worker_id: &str) -> Result<Vec<String>> {
    Target::resolve(worker_id.to_owned())?.skip_to_end()
}

/// One worker being waited on, plus how far into its status log this process has already looked.
///
/// `scanned` is re-synced from the persisted cursor under the lock on every poll and otherwise
/// advances past non-actionable lines in memory only. The cursor file moves solely on delivery, so
/// a wait that dies mid-poll re-reads `working:` lines rather than skipping an undelivered wake.
struct Target {
    id: String,
    dir: Utf8PathBuf,
    status: Utf8PathBuf,
    scanned: u64,
    window_checked_at: Option<Instant>,
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
            window_checked_at: None,
        })
    }

    fn poll(&mut self) -> Result<Option<Outcome>> {
        if !self.dir.exists() {
            return Ok(Some(self.closed(wake::line(
                WakeKind::Closed,
                &format!("worker '{}' directory removed", self.id),
            ))));
        }

        let path = cursor_path(&self.dir);
        let mut cursor = open_cursor(&path)?;
        cursor
            .lock()
            .with_context(|| format!("failed to lock {path}"))?;

        // Another wait may have consumed past our in-memory position since the last poll.
        let persisted = read_cursor(&mut cursor, &path)?;
        self.scanned = self.scanned.max(persisted);

        let Some((kind, line, end)) = self.next_actionable()? else {
            drop(cursor);
            // Only once the log holds nothing further to deliver. A worker can report `done:`
            // and then exit, and that line must still be handed over.
            return self.window_gone_if_confirmed();
        };
        write_cursor(&mut cursor, &path, end)?;
        self.scanned = end;
        drop(cursor);

        Ok(Some(if kind == WakeKind::Closed {
            self.closed(line)
        } else {
            Outcome::Wake {
                id: self.id.clone(),
                line,
            }
        }))
    }

    /// Reads only the bytes appended since the last scan and returns the first actionable line
    /// with the offset just past it. A trailing partial line is left for the next poll.
    fn next_actionable(&mut self) -> Result<Option<(WakeKind, String, u64)>> {
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

        let len = file
            .metadata()
            .with_context(|| format!("failed to inspect {}", self.status))?
            .len();
        // A truncated or replaced log means the offset no longer describes this file.
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

    /// Consumes every actionable line currently in the log and persists the end position.
    fn skip_to_end(&mut self) -> Result<Vec<String>> {
        let path = cursor_path(&self.dir);
        let mut cursor = open_cursor(&path)?;
        cursor
            .lock()
            .with_context(|| format!("failed to lock {path}"))?;
        self.scanned = self.scanned.max(read_cursor(&mut cursor, &path)?);

        let mut skipped = Vec::new();
        while let Some((_, line, end)) = self.next_actionable()? {
            skipped.push(line);
            self.scanned = end;
        }
        // A failed scan leaves `scanned` just past the last complete line, which is where the
        // next wait should resume from.
        write_cursor(&mut cursor, &path, self.scanned)?;
        drop(cursor);
        Ok(skipped)
    }

    /// Reports a worker whose tmux window has gone, so nothing can append to its log again.
    /// Rate-limited, and silent about anything short of a confirmed absence.
    fn window_gone_if_confirmed(&mut self) -> Result<Option<Outcome>> {
        let now = Instant::now();
        if let Some(checked_at) = self.window_checked_at
            && now.duration_since(checked_at) < WINDOW_CHECK_INTERVAL
        {
            return Ok(None);
        }
        self.window_checked_at = Some(now);

        if !worker::window_is_gone(&self.id)? {
            return Ok(None);
        }
        Ok(Some(Outcome::WindowGone {
            id: self.id.clone(),
            status: self.status.clone(),
        }))
    }

    fn closed(&self, line: String) -> Outcome {
        Outcome::Closed {
            id: self.id.clone(),
            status: self.status.clone(),
            line,
        }
    }
}

fn resolve_targets(on: WaitOn) -> Result<Vec<Target>> {
    let ids = match on {
        WaitOn::Workers(ids) => dedup(ids),
        WaitOn::Task(label) => task_worker_ids(&label)?,
    };
    ids.into_iter().map(Target::resolve).collect()
}

fn task_worker_ids(label: &str) -> Result<Vec<String>> {
    worker::validate_task_label(label)?;
    let selection = worker::select_worker_ids_by_task(label)?;
    // A worker with unreadable metadata carries no label, so it cannot be in scope for `label`;
    // surface it but do not let it block a wait on a label it could never carry.
    if !selection.unreadable.is_empty() {
        let ids = selection
            .unreadable
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "warning: worker(s) with unreadable metadata skipped (remove their directories to recover): {ids}"
        );
    }
    if selection.ids.is_empty() {
        bail!("no live workers with task label {label}");
    }
    Ok(selection.ids)
}

fn dedup(ids: Vec<String>) -> Vec<String> {
    let mut deduped: Vec<String> = Vec::new();
    for id in ids {
        if !deduped.contains(&id) {
            deduped.push(id);
        }
    }
    deduped
}

fn timeout_subject(targets: &[Target]) -> String {
    match targets {
        [only] => only.status.to_string(),
        _ => "requested workers".to_owned(),
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

    if rendered.len() <= MAX_LINE_BYTES {
        return rendered;
    }
    let mut cut = MAX_LINE_BYTES;
    while cut > 0 && !rendered.is_char_boundary(cut) {
        cut -= 1;
    }
    rendered.truncate(cut);
    rendered.push_str("... (truncated)");
    rendered
}

pub(crate) fn parse_interval(value: &str) -> Result<Duration, String> {
    let invalid = || {
        format!(
            "wait interval must be a finite positive number at most {MAX_INTERVAL_SECS} seconds"
        )
    };
    let seconds = value.parse::<f64>().map_err(|_| invalid())?;
    if !seconds.is_finite() || seconds <= 0.0 || seconds > MAX_INTERVAL_SECS {
        return Err(invalid());
    }
    Ok(Duration::from_secs_f64(seconds))
}

pub(crate) fn parse_timeout(value: &str) -> Result<Duration, String> {
    const INVALID_TIMEOUT: &str = "wait timeout must be a finite non-negative number";
    let seconds = value
        .parse::<f64>()
        .map_err(|_| INVALID_TIMEOUT.to_owned())?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(INVALID_TIMEOUT.to_owned());
    }
    Ok(Duration::from_secs_f64(seconds))
}
