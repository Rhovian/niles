use std::{
    fs::File,
    io::{self, Write},
};

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::{Serialize, de::DeserializeOwned};

use crate::util::read_optional_string;

pub(crate) fn read_optional_json<T: DeserializeOwned>(path: &Utf8Path) -> Result<Option<T>> {
    let Some(body) = read_optional_string(path)? else {
        return Ok(None);
    };
    serde_json::from_str(&body)
        .with_context(|| format!("failed to parse {path}"))
        .map(Some)
}

pub(crate) fn write_json<T: Serialize + ?Sized>(path: &Utf8Path, value: &T) -> Result<()> {
    let body = serde_json::to_string_pretty(value)
        .with_context(|| format!("failed to serialize {path}"))?;
    atomic_write(path, body)
}

pub(crate) fn read_optional_yaml<T: DeserializeOwned>(path: &Utf8Path) -> Result<Option<T>> {
    let Some(body) = read_optional_string(path)? else {
        return Ok(None);
    };
    parse_yaml(&body)
        .with_context(|| format!("failed to parse {path}"))
        .map(Some)
}

pub(crate) fn write_yaml<T: Serialize>(path: &Utf8Path, value: &T) -> Result<()> {
    let body =
        serde_saphyr::to_string(value).with_context(|| format!("failed to serialize {path}"))?;
    atomic_write(path, body)
}

pub(crate) fn parse_yaml<T: DeserializeOwned>(body: &str) -> Result<T> {
    let options = serde_saphyr::options! {
        with_snippet: false,
    };
    serde_saphyr::from_str_with_options(body, options).map_err(anyhow::Error::new)
}

pub(crate) enum Publication {
    Replace,
    Create,
}

pub(crate) fn publish(
    path: &Utf8Path,
    publication: Publication,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<File> {
    // tempfile 3.27 stages uniquely, cleans up on error and supports both publication modes:
    // https://docs.rs/tempfile/3.27.0/tempfile/struct.NamedTempFile.html#method.persist
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    write(temporary.as_file_mut())?;
    match publication {
        Publication::Replace => temporary.persist(path),
        Publication::Create => temporary.persist_noclobber(path),
    }
    .map_err(|error| error.error)
}

pub(crate) fn atomic_write(path: &Utf8Path, body: impl AsRef<[u8]>) -> Result<()> {
    publish(path, Publication::Replace, |file| {
        file.write_all(body.as_ref())
    })
    .map(drop)
    .with_context(|| format!("failed to write {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::MetadataExt};
    #[test]
    fn writers_publish_whole_files() {
        let dir = crate::test_support::temp_dir("publication");
        for write in [write_json::<&str>, write_yaml::<&str>] {
            let path = dir.join("published");
            fs::write(&path, "old").unwrap();
            let before = fs::metadata(&path).unwrap().ino();
            write(&path, &"new").unwrap();
            assert_ne!(before, fs::metadata(&path).unwrap().ino());
        }
    }
}
