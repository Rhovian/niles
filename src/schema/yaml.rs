use std::{fs, io::ErrorKind};

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::{Serialize, de::DeserializeOwned};
use serde_yaml::Value as YamlValue;

use super::{
    kind::ArtifactKind,
    version::{
        deserialize_failure, malformed_artifact, reject_newer_schema, schema_from_yaml,
        stamp_yaml_value,
    },
};

pub(crate) fn write_yaml<T>(path: &Utf8Path, value: &T) -> Result<()>
where
    T: Serialize + ?Sized,
{
    let mut value = serde_yaml::to_value(value).context("failed to serialize YAML artifact")?;
    stamp_yaml_value(&mut value)?;
    let body = serde_yaml::to_string(&value).context("failed to serialize YAML artifact")?;
    fs::write(path, body).with_context(|| format!("failed to write {path}"))
}

pub(crate) fn read_optional_yaml<T>(path: &Utf8Path, kind: ArtifactKind) -> Result<Option<T>>
where
    T: DeserializeOwned,
{
    let body = match fs::read_to_string(path) {
        Ok(body) => body,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to read {path}")),
    };
    read_yaml_body(path, kind, &body).map(Some)
}

fn read_yaml_body<T>(path: &Utf8Path, kind: ArtifactKind, body: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    let value = parse_yaml_value(path, kind, body)?;
    let probe = schema_from_yaml(&value);
    reject_newer_schema(path, kind, probe)?;
    serde_yaml::from_str(body).map_err(|err| deserialize_failure(path, kind, probe, err))
}

fn parse_yaml_value(path: &Utf8Path, kind: ArtifactKind, body: &str) -> Result<YamlValue> {
    serde_yaml::from_str(body)
        .map_err(|err| anyhow::Error::new(err).context(malformed_artifact(path, kind, "YAML")))
}
