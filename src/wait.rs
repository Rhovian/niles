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
    fs::{self, File},
    io::{ErrorKind, Read, Seek, SeekFrom, Write},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    wake::{self, WakeKind, is_actionable_wake, is_closed_wake},
    worker,
};

pub const EXIT_WAKE: u8 = 0;
pub const EXIT_WORKER_CLOSED: u8 = 10;
pub const EXIT_TIMEOUT: u8 = 22;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// Poll interval used by `niles wait` and by `niles send --wait`.
pub const DEFAULT_INTERVAL_SECS: f64 = 2.0;

/// Largest accepted `--interval`. A poll interval beyond the default timeout is always a typo.
const MAX_INTERVAL_SECS: f64 = 3600.0;

/// Longest status line rendered to the manager. A worker can append a line of any length; the
/// manager's terminal should not have to absorb it.
const MAX_LINE_BYTES: usize = 4096;

const CURSOR_FILE: &str = "status.cursor";

/// How often to ask tmux whether a worker's window still exists. This is a backstop against a
/// wait that would otherwise block for its full timeout, so it does not need to be prompt — and
/// asking on every poll would spend a subprocess per worker per interval for an answer that
/// almost never changes.
const WINDOW_CHECK_INTERVAL: Duration = Duration::from_secs(5);

pub fn wait(
    worker_ids: Vec<String>,
    task: Option<String>,
    interval: f64,
    timeout: Option<f64>,
) -> Result<WaitExit> {
    let mut targets = resolve_targets(worker_ids, task)?;
    let prefix_worker_id = targets.len() > 1;
    let subject = timeout_subject(&targets);

    let interval = positive_seconds_duration(interval, "wait interval")?;
    let timeout = match timeout
        .map(|seconds| non_negative_seconds_duration(seconds, "wait timeout"))
        .transpose()?
    {
        Some(timeout) => timeout,
        None => DEFAULT_TIMEOUT,
    };

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
        let status = worker::status_log_path(&id)?;
        let dir = status
            .parent()
            .map(Utf8Path::to_path_buf)
            .with_context(|| format!("worker status path has no parent directory: {status}"))?;
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
        let guard = CursorLock::acquire(&cursor, &path)?;

        // Another wait may have consumed past our in-memory position since the last poll.
        let persisted = read_cursor(&mut cursor, &path)?;
        self.scanned = self.scanned.max(persisted);

        let Some((line, end)) = self.next_actionable()? else {
            drop(guard);
            // Only once the log holds nothing further to deliver. A worker can report `done:`
            // and then exit, and that line must still be handed over.
            return self.window_gone_if_confirmed();
        };
        write_cursor(&mut cursor, &path, end)?;
        self.scanned = end;
        drop(guard);

        Ok(Some(if is_closed_wake(&line) {
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
    fn next_actionable(&mut self) -> Result<Option<(String, u64)>> {
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

        let mut start = 0usize;
        while let Some(offset) = appended[start..].iter().position(|byte| *byte == b'\n') {
            let raw = &appended[start..start + offset];
            let end = self.scanned + as_u64(start + offset + 1);
            // Offsets come from the raw bytes, so a non-UTF-8 line cannot shift the cursor.
            let line = String::from_utf8_lossy(raw);
            let line = line.trim_end_matches('\r');
            if is_actionable_wake(line) {
                return Ok(Some((render(line), end)));
            }
            start += offset + 1;
        }

        self.scanned += as_u64(start);
        Ok(None)
    }

    /// Consumes every actionable line currently in the log and persists the end position.
    fn skip_to_end(&mut self) -> Result<Vec<String>> {
        let path = cursor_path(&self.dir);
        let mut cursor = open_cursor(&path)?;
        let guard = CursorLock::acquire(&cursor, &path)?;
        self.scanned = self.scanned.max(read_cursor(&mut cursor, &path)?);

        let mut skipped = Vec::new();
        while let Some((line, end)) = self.next_actionable()? {
            skipped.push(line);
            self.scanned = end;
        }
        // A failed scan leaves `scanned` just past the last complete line, which is where the
        // next wait should resume from.
        write_cursor(&mut cursor, &path, self.scanned)?;
        drop(guard);
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

enum Outcome {
    Wake {
        id: String,
        line: String,
    },
    Closed {
        id: String,
        status: Utf8PathBuf,
        line: String,
    },
    WindowGone {
        id: String,
        status: Utf8PathBuf,
    },
    Timeout {
        subject: String,
        timeout: Duration,
    },
}

pub(crate) struct WaitExit {
    code: u8,
    stdout: Option<String>,
    stderr: Option<String>,
}

impl WaitExit {
    fn from_outcome(outcome: Outcome, prefix_worker_id: bool) -> Self {
        match outcome {
            Outcome::Wake { id, line } => Self {
                code: EXIT_WAKE,
                stdout: Some(wake_line(&id, line, prefix_worker_id)),
                stderr: None,
            },
            Outcome::Closed { id, status, line } => Self {
                code: EXIT_WORKER_CLOSED,
                stdout: Some(wake_line(&id, line, prefix_worker_id)),
                stderr: Some(format!(
                    "wait: worker-closed worker={} status={} detail={}",
                    field(&id),
                    field(status.as_str()),
                    detail(&format!("worker '{id}' closed"))
                )),
            },
            Outcome::WindowGone { id, status } => Self {
                code: EXIT_WORKER_CLOSED,
                stdout: Some(wake_line(
                    &id,
                    wake::line(
                        WakeKind::Failed,
                        &format!("worker '{id}' exited without reporting; its window is gone"),
                    ),
                    prefix_worker_id,
                )),
                stderr: Some(format!(
                    "wait: window-gone worker={} status={} detail={}",
                    field(&id),
                    field(status.as_str()),
                    detail(&format!(
                        "worker '{id}' tmux window is gone and its status log ended without a final line"
                    ))
                )),
            },
            Outcome::Timeout { subject, timeout } => Self {
                code: EXIT_TIMEOUT,
                stdout: None,
                stderr: Some(format!(
                    "wait: timeout target={} timeout={}",
                    field(&subject),
                    format_duration(timeout)
                )),
            },
        }
    }

    pub(crate) fn emit(self) -> ExitCode {
        if let Some(stdout) = self.stdout {
            println!("{stdout}");
        }
        if let Some(stderr) = self.stderr {
            eprintln!("{stderr}");
        }
        ExitCode::from(self.code)
    }
}

fn resolve_targets(worker_ids: Vec<String>, task: Option<String>) -> Result<Vec<Target>> {
    let ids = match (worker_ids.is_empty(), task) {
        (false, None) => dedup(worker_ids),
        (true, Some(label)) => task_worker_ids(&label)?,
        (false, Some(_)) => bail!("use either --worker <id> or --task <label>, not both"),
        (true, None) => bail!("wait requires --worker <id> or --task <label>"),
    };

    let mut targets = Vec::with_capacity(ids.len());
    for id in ids {
        targets.push(Target::resolve(id)?);
    }
    if targets.is_empty() {
        bail!("wait requires at least one worker target");
    }
    Ok(targets)
}

fn task_worker_ids(label: &str) -> Result<Vec<String>> {
    worker::validate_task_label(label)?;
    let selection = worker::select_worker_ids_by_task(label)?;
    if !selection.failures.is_empty() {
        let failures = selection
            .failures
            .into_iter()
            .map(|(id, err)| format!("{id}: {err}"))
            .collect::<Vec<_>>()
            .join("; ");
        bail!("failed to select workers with task label {label}: {failures}");
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

fn cursor_path(dir: &Utf8Path) -> Utf8PathBuf {
    dir.join(CURSOR_FILE)
}

/// Opens the cursor for locking and rewriting. `O_NOFOLLOW` so a symlink planted at the cursor
/// path cannot redirect the write out of the worker directory.
fn open_cursor(path: &Utf8Path) -> Result<File> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("failed to open {path}"))
}

/// An exclusive advisory lock over the read-scan-advance window, released on drop and by the
/// kernel if this process dies holding it.
struct CursorLock<'a> {
    fd: i32,
    path: &'a Utf8Path,
}

impl<'a> CursorLock<'a> {
    fn acquire(file: &File, path: &'a Utf8Path) -> Result<Self> {
        let fd = file.as_raw_fd();
        // SAFETY: `fd` is owned by `file`, which outlives the returned guard.
        if unsafe { libc::flock(fd, libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("failed to lock {path}"));
        }
        Ok(Self { fd, path })
    }
}

impl Drop for CursorLock<'_> {
    fn drop(&mut self) {
        // SAFETY: the fd is still open; the file this borrows from outlives the guard.
        if unsafe { libc::flock(self.fd, libc::LOCK_UN) } != 0 {
            eprintln!("wait: failed to unlock {}", self.path);
        }
    }
}

fn read_cursor(file: &mut File, path: &Utf8Path) -> Result<u64> {
    file.seek(SeekFrom::Start(0))
        .with_context(|| format!("failed to read {path}"))?;
    let mut body = String::new();
    file.read_to_string(&mut body)
        .with_context(|| format!("failed to read {path}"))?;
    let body = body.trim();
    // A cursor this process just created is empty, which is the same position as zero.
    if body.is_empty() {
        return Ok(0);
    }
    body.parse::<u64>()
        .with_context(|| format!("invalid wake cursor in {path}; remove it to resume"))
}

fn write_cursor(file: &mut File, path: &Utf8Path, offset: u64) -> Result<()> {
    let body = format!("{offset}\n");
    file.set_len(0)
        .with_context(|| format!("failed to write {path}"))?;
    file.seek(SeekFrom::Start(0))
        .with_context(|| format!("failed to write {path}"))?;
    file.write_all(body.as_bytes())
        .with_context(|| format!("failed to write {path}"))
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

fn wake_line(id: &str, line: String, prefix_worker_id: bool) -> String {
    if prefix_worker_id {
        return format!("{id}: {line}");
    }
    line
}

fn positive_seconds_duration(seconds: f64, label: &str) -> Result<Duration> {
    if !seconds.is_finite() || seconds <= 0.0 {
        bail!("{label} must be a finite positive number");
    }
    if seconds > MAX_INTERVAL_SECS {
        bail!("{label} must be at most {MAX_INTERVAL_SECS} seconds");
    }
    Ok(Duration::from_secs_f64(seconds))
}

fn non_negative_seconds_duration(seconds: f64, label: &str) -> Result<Duration> {
    if !seconds.is_finite() || seconds < 0.0 {
        bail!("{label} must be a finite non-negative number");
    }
    if seconds == 0.0 {
        Ok(Duration::ZERO)
    } else {
        Ok(Duration::from_secs_f64(seconds))
    }
}

fn format_duration(duration: Duration) -> String {
    let nanos = duration.as_nanos();
    if nanos == 0 {
        return "0s".to_owned();
    }
    if nanos.is_multiple_of(1_000_000_000) {
        return format!("{}s", duration.as_secs());
    }
    if nanos.is_multiple_of(1_000_000) {
        return format!("{}ms", nanos / 1_000_000);
    }
    format!("{:.3}s", duration.as_secs_f64())
}

fn field(value: &str) -> String {
    if value.is_empty()
        || value
            .chars()
            .any(|ch| ch.is_whitespace() || ch == '\'' || ch == '"')
    {
        quoted(value)
    } else {
        value.to_owned()
    }
}

fn quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn detail(value: &str) -> String {
    match serde_json::to_string(value) {
        Ok(json) => json,
        Err(_) => quoted(value),
    }
}

/// Byte offsets within a status log start life as `usize` from slice scanning. Rust has no
/// target where `usize` exceeds 64 bits, so widening is lossless and there is no default to pick.
fn as_u64(value: usize) -> u64 {
    value as u64
}
