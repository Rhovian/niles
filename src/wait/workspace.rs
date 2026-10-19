use std::{
    fs::{self, File, TryLockError},
    io::{self, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use camino::Utf8PathBuf;

use super::{Target, WaitExit, cursor::*, exit::Outcome, render};
use crate::{store, util, wake::WakeKind, worker};

const TAKEOVER_POLL: Duration = Duration::from_millis(100);

/// libc 0.2.189 exposes flock as an fd + operation (src/unix/mod.rs:2221); it
/// cannot replace a live holder. Rust 1.89 File::lock has the same exclusion model.
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
        let temporary = root.join(format!("wait.running.{}", uuid::Uuid::new_v4()));
        let presence = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)
            .with_context(|| format!("failed to create {temporary}"))?;
        presence.lock_shared()?;
        // A stale waiter keeps only the unlinked old inode, never current presence.
        fs::rename(&temporary, &running).with_context(|| format!("failed to publish {running}"))?;
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
    let deadline = Instant::now() + timeout;
    let mut next_poll = Instant::now();
    loop {
        // Keep takeover serialized with printing, including the cursor writes that follow it.
        let gate = open_lock_file(&registration.path)?;
        gate.lock()?;
        if !registration.current()? {
            return Ok(WaitExit::from_outcome(Outcome::Superseded));
        }
        if Instant::now() >= next_poll {
            let mut delivered = false;
            let stdout = io::stdout();
            let mut output = stdout.lock();
            for worker in worker::live_worker_locations(&workspace)? {
                let mut target = Target {
                    id: worker.id,
                    status: crate::wake::status_log_path(&worker.worker_dir),
                    dir: worker.worker_dir,
                    scanned: 0,
                };
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
                target.scanned = read_cursor(&mut cursor, &path)?;
                if let Some((kind, line, end)) = target.next_actionable()? {
                    if kind == WakeKind::Closed {
                        continue;
                    }
                    let line = render(&format!("{} {line}", target.id));
                    writeln!(output, "{line}").context("failed to print workspace wake")?;
                    output.flush().context("failed to flush workspace wake")?;
                    // A killed or broken-output wait cannot consume an unprinted line.
                    write_cursor(&mut cursor, &path, end)?;
                    delivered = true;
                }
            }
            if delivered {
                return Ok(WaitExit::from_outcome(Outcome::Wake));
            }
            next_poll = Instant::now() + interval;
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
