use anyhow::{Result, bail};
use camino::Utf8Path;
use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;

use super::{kind::ArtifactKind, status::SchemaStatus};

pub(crate) const CURRENT_SCHEMA: u64 = 2;
const LEGACY_SCHEMA: u64 = 1;
const FIELD: &str = "niles_schema";

pub(in crate::schema) fn stamp_json_value(value: &mut JsonValue) -> Result<()> {
    let Some(object) = value.as_object_mut() else {
        bail!("schema-stamped JSON artifacts must serialize as an object");
    };
    object.insert(FIELD.to_owned(), JsonValue::from(CURRENT_SCHEMA));
    Ok(())
}

pub(in crate::schema) fn stamp_yaml_value(value: &mut YamlValue) -> Result<()> {
    let YamlValue::Mapping(object) = value else {
        bail!("schema-stamped YAML artifacts must serialize as a mapping");
    };
    object.insert(
        YamlValue::String(FIELD.to_owned()),
        YamlValue::from(CURRENT_SCHEMA),
    );
    Ok(())
}

pub(in crate::schema) fn schema_from_json(value: &JsonValue) -> SchemaProbe {
    let Some(object) = value.as_object() else {
        return SchemaProbe::Invalid;
    };
    match object.get(FIELD) {
        None => SchemaProbe::Schema(LEGACY_SCHEMA),
        Some(value) => value
            .as_u64()
            .map_or(SchemaProbe::Invalid, SchemaProbe::Schema),
    }
}

pub(in crate::schema) fn schema_from_yaml(value: &YamlValue) -> SchemaProbe {
    let YamlValue::Mapping(object) = value else {
        return SchemaProbe::Invalid;
    };
    let key = YamlValue::String(FIELD.to_owned());
    match object.get(&key) {
        None => SchemaProbe::Schema(LEGACY_SCHEMA),
        Some(value) => value
            .as_u64()
            .map_or(SchemaProbe::Invalid, SchemaProbe::Schema),
    }
}

pub(in crate::schema) fn reject_newer_schema(
    path: &Utf8Path,
    kind: ArtifactKind,
    probe: SchemaProbe,
) -> Result<()> {
    match probe {
        SchemaProbe::Schema(schema) if schema <= CURRENT_SCHEMA => Ok(()),
        SchemaProbe::Schema(schema) => {
            bail!(
                "{} {path} was written by a newer niles (schema {schema}, this binary expects {}); upgrade this binary, or use the newer binary that wrote it",
                kind.label(),
                CURRENT_SCHEMA
            )
        }
        SchemaProbe::Invalid => Ok(()),
    }
}

pub(in crate::schema) fn deserialize_failure<E>(
    path: &Utf8Path,
    kind: ArtifactKind,
    probe: SchemaProbe,
    err: E,
) -> anyhow::Error
where
    E: std::error::Error + Send + Sync + 'static,
{
    let message = deserialize_failure_message(path, kind, probe);
    if matches!(probe, SchemaProbe::Schema(CURRENT_SCHEMA)) {
        anyhow::Error::new(err).context(message)
    } else {
        anyhow::anyhow!(message)
    }
}

fn deserialize_failure_message(path: &Utf8Path, kind: ArtifactKind, probe: SchemaProbe) -> String {
    match probe {
        SchemaProbe::Schema(schema) if schema < CURRENT_SCHEMA => {
            format!(
                "{} {path} was written by an older niles (schema {schema}, this binary expects {}) and could not be read as the current format; {}",
                kind.label(),
                CURRENT_SCHEMA,
                kind.remediation()
            )
        }
        SchemaProbe::Schema(CURRENT_SCHEMA) => {
            format!(
                "{} {path} declares schema {}, but does not match this binary's expected format; {}",
                kind.label(),
                CURRENT_SCHEMA,
                kind.remediation()
            )
        }
        SchemaProbe::Schema(schema) => {
            format!(
                "{} {path} was written by a newer niles (schema {schema}, this binary expects {}); upgrade this binary, or use the newer binary that wrote it",
                kind.label(),
                CURRENT_SCHEMA
            )
        }
        SchemaProbe::Invalid => {
            format!(
                "{} {path} has an invalid {FIELD} stamp and could not be read as the current format (this binary expects schema {}); {}",
                kind.label(),
                CURRENT_SCHEMA,
                kind.remediation()
            )
        }
    }
}

pub(in crate::schema) fn malformed_artifact(
    path: &Utf8Path,
    kind: ArtifactKind,
    format: &str,
) -> String {
    format!(
        "{} {path} is malformed {format}; schema is unknown and this binary expects schema {}; {}",
        kind.label(),
        CURRENT_SCHEMA,
        kind.remediation()
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::schema) enum SchemaProbe {
    Schema(u64),
    Invalid,
}

impl SchemaProbe {
    pub(in crate::schema) fn into_status(self) -> SchemaStatus {
        match self {
            SchemaProbe::Schema(CURRENT_SCHEMA) => SchemaStatus::Current(CURRENT_SCHEMA),
            SchemaProbe::Schema(schema) if schema < CURRENT_SCHEMA => SchemaStatus::Older(schema),
            SchemaProbe::Schema(schema) => SchemaStatus::Newer(schema),
            SchemaProbe::Invalid => SchemaStatus::Invalid,
        }
    }
}
