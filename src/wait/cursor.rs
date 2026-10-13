use std::{
    fs::{self, File, TryLockError},
    io::{ErrorKind, Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

const CURSOR_FILE: &str = "status.cursor";
const WAITER_FILE: &str = "status.waiter";
const WAITING_FILE: &str = "status.waiting";

pub(crate) fn cursor_path(dir: &Utf8Path) -> Utf8PathBuf {
    dir.join(CURSOR_FILE)
}

/// The empty file whose `flock` is a worker's wait claim. It is never written.
pub(super) fn waiter_path(dir: &Utf8Path) -> Utf8PathBuf {
    dir.join(WAITER_FILE)
}

/// The empty file a running wait holds a shared `flock` on, so others can tell a wait is running.
///
/// Separate from the claim because any probe of the claim takes it for an instant, and a wait that
/// started in that instant would be refused as already waiting. A probe of this file only delays
/// such a wait by that instant.
pub(super) fn waiting_path(dir: &Utf8Path) -> Utf8PathBuf {
    dir.join(WAITING_FILE)
}

/// Whether a `niles wait` is running on the worker in `dir`.
pub(crate) fn wait_running(dir: &Utf8Path) -> Result<bool> {
    let path = waiting_path(dir);
    // Opened without `create`: a worker no wait has ever run on has no file, and a reader has no
    // business leaving one behind.
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("failed to open {path}")),
    };
    match file.try_lock() {
        Ok(()) => Ok(false),
        Err(TryLockError::WouldBlock) => Ok(true),
        Err(TryLockError::Error(err)) => Err(err).with_context(|| format!("failed to lock {path}")),
    }
}

/// Opens a worker's lock file (the cursor, the wait claim and presence, the check-in lock). `O_NOFOLLOW` keeps a
/// symlink planted at the path from redirecting the open out of the worker directory.
pub(crate) fn open_lock_file(path: &Utf8Path) -> Result<File> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("failed to open {path}"))
}

pub(super) fn read_cursor(file: &mut File, path: &Utf8Path) -> Result<u64> {
    file.seek(SeekFrom::Start(0))
        .with_context(|| format!("failed to read {path}"))?;
    let mut body = String::new();
    file.read_to_string(&mut body)
        .with_context(|| format!("failed to read {path}"))?;
    parse_cursor(&body, path)
}

pub(crate) fn parse_cursor(body: &str, path: &Utf8Path) -> Result<u64> {
    let body = body.trim();
    if body.is_empty() {
        return Ok(0);
    }
    body.parse::<u64>()
        .with_context(|| format!("invalid wake cursor in {path}; remove it to resume"))
}

pub(super) fn write_cursor(file: &mut File, path: &Utf8Path, offset: u64) -> Result<()> {
    let body = format!("{offset}\n");
    file.set_len(0)
        .with_context(|| format!("failed to write {path}"))?;
    file.seek(SeekFrom::Start(0))
        .with_context(|| format!("failed to write {path}"))?;
    file.write_all(body.as_bytes())
        .with_context(|| format!("failed to write {path}"))
}
