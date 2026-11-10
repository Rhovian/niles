use super::cursor::*;
use super::exit::Outcome;
use super::{Target, WaitExit, render};
use crate::wake::{self, WakeKind};
use crate::{store, util, worker};
use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, TryLockError};
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::thread;
use std::time::{Duration, Instant};

const TAKEOVER_POLL: Duration = Duration::from_millis(100);

/// Replacing the presence inode under a short-lived lock makes takeover and output atomic.
struct Registration {
    path: Utf8PathBuf,
    running: Utf8PathBuf,
    presence: File,
}

impl Registration {
    fn start() -> Result<Self> {
        let root = store::workers_dir(&util::current_dir_utf8()?)?;
        fs::create_dir_all(&root).with_context(|| format!("failed to create {root}"))?;
        let path = root.join("wait.lock");
        let gate = open_lock_file(&path)?;
        gate.lock()?;
        let running = root.join("wait.running");
        // Refuse a planted symlink just as for the workspace and cursor locks.
        drop(open_lock_file(&running)?);
        // A stale waiter keeps only the unlinked old inode, never current presence.
        let presence = store::publish(&running, store::Publication::Replace, |file| {
            file.lock_shared()
        })?;
        Ok(Self {
            path,
            running,
            presence,
        })
    }

    fn current(&self) -> Result<bool> {
        let current = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&self.running)
            .with_context(|| format!("failed to open {}", self.running))?
            .metadata()?;
        let own = self.presence.metadata()?;
        // The held descriptor prevents this inode from being reused during our lifetime.
        Ok(current.dev() == own.dev() && current.ino() == own.ino())
    }
}

pub(crate) fn wait(interval: Duration, timeout: Duration) -> Result<WaitExit> {
    let registration = Registration::start()?;
    let workspace = util::current_dir_utf8()?;
    let deadline = Instant::now()
        .checked_add(timeout)
        .context("wait deadline overflow")?;
    let mut next_poll = Instant::now();
    // Rust 1.89 File::open/seek (std/fs.rs) do not retain offsets across reopened files.
    // Remember progress here without persisting past unprinted reports.
    let mut targets = BTreeMap::new();
    loop {
        // Keep takeover serialized with printing, including the cursor writes that follow it.
        let gate = open_lock_file(&registration.path)?;
        gate.lock()?;
        if !registration.current()? {
            return Ok(WaitExit::from_outcome(Outcome::Superseded));
        }
        if Instant::now() >= next_poll {
            if poll_workers(&workspace, &mut targets, &mut io::stdout().lock())? {
                return Ok(WaitExit::from_outcome(Outcome::Wake));
            }
            next_poll = Instant::now()
                .checked_add(interval)
                .context("poll deadline overflow")?;
        }
        if Instant::now() >= deadline {
            return Ok(WaitExit::from_outcome(Outcome::Timeout { timeout }));
        }
        drop(gate);
        let now = Instant::now();
        thread::sleep(
            TAKEOVER_POLL
                .min(next_poll.saturating_duration_since(now))
                .min(deadline.saturating_duration_since(now)),
        );
    }
}

fn poll_workers(
    workspace: &Utf8Path,
    targets: &mut BTreeMap<String, Target>,
    output: &mut impl Write,
) -> Result<bool> {
    let workers = worker::live_worker_locations(workspace)?;
    let live: BTreeSet<_> = workers.iter().map(|worker| worker.id.as_str()).collect();
    targets.retain(|id, _| live.contains(id.as_str()));
    let mut delivered = false;
    for worker in &workers {
        let target = targets
            .entry(worker.id.clone())
            .or_insert_with(|| Target::new(worker.id.clone(), worker.worker_dir.clone()));
        let path = cursor_path(&target.dir);
        let mut cursor = match open_lock_file(&path) {
            Ok(cursor) => cursor,
            Err(_) if !target.dir.exists() => continue,
            Err(error) => return Err(error),
        };
        // A busy sender owns its old lines. Never hold up takeover on its cursor.
        match cursor.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => continue,
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
        let persisted = read_cursor(&mut cursor, &path)?;
        let mut report = target.next_report(persisted)?;
        if report.is_none()
            && (target.unreported || persisted == 0)
            && target.liveness.due(Instant::now())
        {
            let health = worker::wait_health(&target.dir, chrono::Utc::now())
                .with_context(|| format!("failed to probe worker {}", target.id))?;
            if let Some(detail) = target.liveness.observe(health, Instant::now()) {
                // Activity probing may have taken time; an actual worker report wins.
                report = target.next_report(persisted)?;
                if report.is_none() {
                    util::append_line(&target.status, &wake::line(WakeKind::Gone, detail))?;
                    report = target.next_report(persisted)?;
                }
            }
        }
        if let Some((line, end)) = report {
            let line = render(&format!("{} {line}", target.id));
            writeln!(output, "{line}").context("failed to print workspace wake")?;
            output.flush().context("failed to flush workspace wake")?;
            // A killed or broken-output wait cannot consume an unprinted line.
            write_cursor(&mut cursor, &path, end)?;
            target.scanned = end;
            delivered = true;
        }
    }
    Ok(delivered)
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
