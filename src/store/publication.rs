use anyhow::{Context, Result};
use std::{
    fs::File,
    io::{self, Write},
    path::Path,
};

pub(crate) enum Publication {
    Replace,
    Create,
}

pub(crate) fn publish(
    path: impl AsRef<Path>,
    publication: Publication,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<File> {
    // tempfile 3.27 stages uniquely, cleans up on error and supports both publication modes:
    // https://docs.rs/tempfile/3.27.0/tempfile/struct.NamedTempFile.html#method.persist
    let path = path.as_ref();
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    write(temporary.as_file_mut())?;
    match publication {
        Publication::Replace => temporary.persist(path),
        Publication::Create => temporary.persist_noclobber(path),
    }
    .map_err(|error| error.error)
}

pub(crate) fn atomic_write(path: impl AsRef<Path>, body: impl AsRef<[u8]>) -> Result<()> {
    let path = path.as_ref();
    publish(path, Publication::Replace, |file| {
        file.write_all(body.as_ref())
    })
    .map(drop)
    .with_context(|| format!("failed to write {}", path.display()))
}
