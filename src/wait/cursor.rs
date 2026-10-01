use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

const CURSOR_FILE: &str = "status.cursor";

pub(crate) fn cursor_path(dir: &Utf8Path) -> Utf8PathBuf {
    dir.join(CURSOR_FILE)
}

/// Opens the cursor for locking and rewriting. `O_NOFOLLOW` keeps a symlink planted at the cursor
/// path from redirecting the write out of the worker directory.
pub(super) fn open_cursor(path: &Utf8Path) -> Result<File> {
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
