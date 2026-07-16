use std::{fs, io::ErrorKind};

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value as JsonValue;

use super::{
    kind::ArtifactKind,
    version::{
        deserialize_failure, malformed_artifact, reject_newer_schema, schema_from_json,
        stamp_json_value,
    },
};

pub(crate) fn write_json<T>(path: &Utf8Path, value: &T) -> Result<()>
where
    T: Serialize + ?Sized,
{
    let mut value = serde_json::to_value(value).context("failed to serialize JSON artifact")?;
    stamp_json_value(&mut value)?;
    let body = serde_json::to_string_pretty(&value).context("failed to serialize JSON artifact")?;
    fs::write(path, body).with_context(|| format!("failed to write {path}"))
}

#[cfg(test)]
pub(crate) fn read_json<T>(path: &Utf8Path, kind: ArtifactKind) -> Result<T>
where
    T: DeserializeOwned,
{
    let body = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    read_json_body(path, kind, &body)
}

pub(crate) fn read_optional_json<T>(path: &Utf8Path, kind: ArtifactKind) -> Result<Option<T>>
where
    T: DeserializeOwned,
{
    let body = match fs::read_to_string(path) {
        Ok(body) => body,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to read {path}")),
    };
    read_json_body(path, kind, &body).map(Some)
}

fn read_json_body<T>(path: &Utf8Path, kind: ArtifactKind, body: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    let value = parse_json_value(path, kind, body)?;
    let probe = schema_from_json(&value);
    reject_newer_schema(path, kind, probe)?;
    serde_json::from_value(value).map_err(|err| deserialize_failure(path, kind, probe, err))
}

fn parse_json_value(path: &Utf8Path, kind: ArtifactKind, body: &str) -> Result<JsonValue> {
    serde_json::from_str(body)
        .map_err(|err| anyhow::Error::new(err).context(malformed_artifact(path, kind, "JSON")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::test_support::temp_test_path;

    #[derive(Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
    struct Example {
        value: String,
    }

    #[test]
    fn stamped_json_writes_and_reads_current_schema() {
        let root = temp_test_path("json-current");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("artifact.json");

        write_json(
            &path,
            &Example {
                value: "ok".to_owned(),
            },
        )
        .unwrap();

        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains(r#""niles_schema": 2"#));
        assert_eq!(
            read_json::<Example>(&path, ArtifactKind::WorkerMetadata).unwrap(),
            Example {
                value: "ok".to_owned()
            }
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_json_reads_when_current_shape_matches() {
        let root = temp_test_path("json-legacy-ok");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("state.json");
        fs::write(&path, r#"{"value":"ok"}"#).unwrap();

        assert_eq!(
            read_json::<Example>(&path, ArtifactKind::WorkerMetadata).unwrap(),
            Example {
                value: "ok".to_owned()
            }
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_json_errors_only_after_typed_deserialization_fails() {
        let root = temp_test_path("json-legacy");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("state.json");
        fs::write(&path, r#"{"id":"old"}"#).unwrap();

        let err = read_json::<Example>(&path, ArtifactKind::WorkerMetadata).unwrap_err();
        let err = err.to_string();

        assert!(err.contains("worker metadata"));
        assert!(err.contains("schema 1"));
        assert!(err.contains("expects 2"));
        assert!(err.contains("remove the worker dir"));
        assert!(!err.contains("missing field"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn current_schema_deserialization_error_keeps_serde_source() {
        let root = temp_test_path("json-current-corrupt");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("state.json");
        fs::write(&path, r#"{"niles_schema":2,"id":"bad"}"#).unwrap();

        let err = read_json::<Example>(&path, ArtifactKind::WorkerMetadata).unwrap_err();
        let chain = err.chain().map(|err| err.to_string()).collect::<Vec<_>>();

        assert!(chain[0].contains("declares schema 2"));
        assert!(chain.iter().any(|err| err.contains("missing field")));

        fs::remove_dir_all(root).unwrap();
    }
}
