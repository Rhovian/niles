use std::{fs, io::ErrorKind};

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value as JsonValue;

use super::{
    kind::ArtifactKind,
    version::{
        CURRENT_SCHEMA, SchemaProbe, deserialize_failure, malformed_artifact,
        reject_incompatible_schema, schema_from_json,
    },
};

/// A schema-stamped YAML artifact.
///
/// Implement this only for structs with named fields: the writer flattens the artifact so it can
/// append the schema stamp as the final top-level mapping key.
pub(crate) trait YamlArtifact: Serialize {}

#[derive(Serialize)]
struct StampedArtifact<'a, T: ?Sized> {
    #[serde(flatten)]
    artifact: &'a T,
    niles_schema: u64,
}

pub(crate) fn parse_yaml<T>(body: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    deserialize_yaml(body).map_err(anyhow::Error::new)
}

pub(crate) fn write_yaml<T>(path: &Utf8Path, value: &T) -> Result<()>
where
    T: YamlArtifact + ?Sized,
{
    let stamped = StampedArtifact {
        artifact: value,
        niles_schema: CURRENT_SCHEMA,
    };
    let body = serde_saphyr::to_string(&stamped).context("failed to serialize YAML artifact")?;
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
    let probe = probe_schema(body)
        .map_err(|err| anyhow::Error::new(err).context(malformed_artifact(path, kind, "YAML")))?;
    reject_incompatible_schema(path, kind, probe)?;
    deserialize_yaml(body).map_err(|err| deserialize_failure(path, kind, probe, err))
}

pub(in crate::schema) fn probe_schema(
    body: &str,
) -> std::result::Result<SchemaProbe, serde_saphyr::Error> {
    deserialize_yaml::<JsonValue>(body).map(|value| schema_from_json(&value))
}

fn deserialize_yaml<T>(body: &str) -> std::result::Result<T, serde_saphyr::Error>
where
    T: DeserializeOwned,
{
    let options = serde_saphyr::options! {
        with_snippet: false,
    };
    serde_saphyr::from_str_with_options(body, options)
}
