use super::Origin;
use anyhow::{Context, Result, ensure};
use camino::Utf8Path;
use std::{
    fs,
    io::{ErrorKind, Read},
    os::unix::fs::OpenOptionsExt,
};

const MAX_DOCUMENT_BYTES: usize = 131_072;

pub(crate) fn exists(path: &Utf8Path, origin: Origin) -> Result<bool> {
    if origin == Origin::Workspace {
        let dir = path.parent().context("source file has no parent")?;
        match fs::symlink_metadata(dir) {
            Ok(meta) => ensure!(
                meta.is_dir(),
                "source directory must be a directory without symlinks: {dir}"
            ),
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err).with_context(|| format!("failed to inspect {dir}")),
        }
    }
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(
                origin != Origin::Workspace || meta.is_file(),
                "source must be a regular file without symlinks: {path}"
            );
            Ok(true)
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("failed to inspect {path}")),
    }
}

pub(crate) fn read(path: &Utf8Path, origin: Origin) -> Result<String> {
    let flags = if origin == Origin::Workspace {
        libc::O_NOFOLLOW
    } else {
        0
    };
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | flags)
        .open(path)
        .with_context(|| format!("failed to open {path}"))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect {path}"))?;
    ensure!(metadata.is_file(), "source must be a regular file: {path}");
    let mut bytes = Vec::new();
    file.take((MAX_DOCUMENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {path}"))?;
    ensure!(
        bytes.len() <= MAX_DOCUMENT_BYTES,
        "source file {path} read {} bytes (file size {} bytes); maximum is {MAX_DOCUMENT_BYTES} bytes",
        bytes.len(),
        metadata.len()
    );
    String::from_utf8(bytes).with_context(|| format!("source file is not UTF-8: {path}"))
}
