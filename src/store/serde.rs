use std::{fs, io::ErrorKind};

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::{Serialize, de::DeserializeOwned};

pub(crate) fn read_optional_json<T: DeserializeOwned>(path: &Utf8Path) -> Result<Option<T>> {
    let body = match fs::read_to_string(path) {
        Ok(body) => body,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to read {path}")),
    };
    serde_json::from_str(&body)
        .with_context(|| format!("failed to parse {path}"))
        .map(Some)
}

pub(crate) fn write_json<T: Serialize + ?Sized>(path: &Utf8Path, value: &T) -> Result<()> {
    let body = serde_json::to_string_pretty(value)
        .with_context(|| format!("failed to serialize {path}"))?;
    fs::write(path, body).with_context(|| format!("failed to write {path}"))
}

pub(crate) fn read_optional_yaml<T: DeserializeOwned>(path: &Utf8Path) -> Result<Option<T>> {
    let body = match fs::read_to_string(path) {
        Ok(body) => body,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to read {path}")),
    };
    parse_yaml(&body)
        .with_context(|| format!("failed to parse {path}"))
        .map(Some)
}

pub(crate) fn write_yaml<T: Serialize>(path: &Utf8Path, value: &T) -> Result<()> {
    let body =
        serde_saphyr::to_string(value).with_context(|| format!("failed to serialize {path}"))?;
    fs::write(path, body).with_context(|| format!("failed to write {path}"))
}

pub(crate) fn parse_yaml<T: DeserializeOwned>(body: &str) -> Result<T> {
    let options = serde_saphyr::options! {
        with_snippet: false,
    };
    serde_saphyr::from_str_with_options(body, options).map_err(anyhow::Error::new)
}
