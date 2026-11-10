use super::atomic_write;
use crate::util::read_optional_string;
use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::Serialize;
use serde::de::DeserializeOwned;

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

#[cfg(test)]
mod tests {

    use super::*;
    use crate::test_support;
    use std::{fs, os::unix::fs::MetadataExt};
    #[test]
    fn writers_publish_whole_files() {
        let dir = test_support::temp_dir("publication");
        for write in [write_json::<&str>, write_yaml::<&str>] {
            let path = dir.join("published");
            atomic_write(&path, "old").unwrap();
            let before = fs::metadata(&path).unwrap().ino();
            write(&path, &"new").unwrap();
            assert_ne!(before, fs::metadata(&path).unwrap().ino());
        }
    }
}
