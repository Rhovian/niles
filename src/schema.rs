use std::{fs, io::ErrorKind};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map as JsonMap, Value as JsonValue};
use serde_yaml::{Mapping as YamlMap, Value as YamlValue};

pub(crate) const CURRENT_SCHEMA: u64 = 2;
const LEGACY_SCHEMA: u64 = 1;
const FIELD: &str = "niles_schema";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ArtifactKind {
    CapabilityManifest,
    Directory,
    GlobalIndex,
    ManagerSession,
    RunPlan,
    RunPointer,
    RunState,
    StepRecord,
    WorkerMetadata,
    WorkerPointer,
    WorkspaceManifest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SchemaStatus {
    Current(u64),
    Older(u64),
    Newer(u64),
    Invalid,
    Malformed,
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchemaObservation {
    pub(crate) kind: ArtifactKind,
    pub(crate) path: Utf8PathBuf,
    pub(crate) status: SchemaStatus,
}

pub(crate) fn write_json<T>(path: &Utf8Path, value: &T) -> Result<()>
where
    T: Serialize + ?Sized,
{
    let mut value = serde_json::to_value(value).context("failed to serialize JSON artifact")?;
    stamp_json_value(&mut value)?;
    let body = serde_json::to_string_pretty(&value).context("failed to serialize JSON artifact")?;
    fs::write(path, body).with_context(|| format!("failed to write {path}"))
}

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

pub(crate) fn read_json_value_as<T>(path: &Utf8Path, kind: ArtifactKind) -> Result<JsonValue>
where
    T: DeserializeOwned,
{
    let body = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    let value = parse_json_value(path, kind, &body)?;
    let probe = schema_from_json(&value);
    reject_newer_schema(path, kind, probe)?;
    serde_json::from_value::<T>(value.clone())
        .map_err(|err| deserialize_failure(path, kind, probe, err))?;
    Ok(value)
}

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

pub(crate) fn inspect_json(path: &Utf8Path, kind: ArtifactKind) -> SchemaObservation {
    let status = match fs::read_to_string(path) {
        Ok(body) => match serde_json::from_str::<JsonValue>(&body) {
            Ok(value) => schema_from_json(&value).into_status(),
            Err(_) => SchemaStatus::Malformed,
        },
        Err(_) => SchemaStatus::Unreadable,
    };
    SchemaObservation {
        kind,
        path: path.to_path_buf(),
        status,
    }
}

pub(crate) fn inspect_yaml(path: &Utf8Path, kind: ArtifactKind) -> SchemaObservation {
    let status = match fs::read_to_string(path) {
        Ok(body) => match serde_yaml::from_str::<YamlValue>(&body) {
            Ok(value) => schema_from_yaml(&value).into_status(),
            Err(_) => SchemaStatus::Malformed,
        },
        Err(_) => SchemaStatus::Unreadable,
    };
    SchemaObservation {
        kind,
        path: path.to_path_buf(),
        status,
    }
}

pub(crate) fn scan_workspace(root: &Utf8Path) -> Result<Vec<SchemaObservation>> {
    let niles = root.join(".niles");
    if !niles.exists() {
        return Ok(Vec::new());
    }

    let mut observations = Vec::new();
    push_yaml_if_file(
        &mut observations,
        niles.join("manifest.yaml"),
        ArtifactKind::WorkspaceManifest,
    );

    let runs = niles.join("runs");
    for path in read_dir_paths(&mut observations, &runs) {
        if path.is_dir() {
            push_json_if_file(
                &mut observations,
                path.join("state.json"),
                ArtifactKind::RunState,
            );
            push_json_if_file(
                &mut observations,
                path.join("plan.json"),
                ArtifactKind::RunPlan,
            );
            for step_path in read_dir_paths(&mut observations, &path.join("steps")) {
                if step_path.extension() == Some("json") {
                    push_json_if_file(&mut observations, step_path, ArtifactKind::StepRecord);
                }
            }
        } else if path.extension() == Some("json") {
            push_json_if_file(&mut observations, path, ArtifactKind::RunPointer);
        }
    }

    let workers = niles.join("worker");
    for path in read_dir_paths(&mut observations, &workers) {
        if path.is_dir() {
            push_json_if_file(
                &mut observations,
                path.join("meta.json"),
                ArtifactKind::WorkerMetadata,
            );
        } else if path.extension() == Some("json") {
            push_json_if_file(&mut observations, path, ArtifactKind::WorkerPointer);
        }
    }

    let sessions = niles.join("sessions");
    for path in read_dir_paths(&mut observations, &sessions) {
        if path.is_dir() {
            push_json_if_file(
                &mut observations,
                path.join("session.json"),
                ArtifactKind::ManagerSession,
            );
        }
    }

    let capabilities = niles.join("capabilities");
    for path in read_dir_paths(&mut observations, &capabilities) {
        if path.extension() == Some("json") {
            push_json_if_file(&mut observations, path, ArtifactKind::CapabilityManifest);
        }
    }

    observations.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.kind.cmp(&right.kind))
    });
    Ok(observations)
}

impl ArtifactKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ArtifactKind::CapabilityManifest => "capability manifest",
            ArtifactKind::Directory => "artifact directory",
            ArtifactKind::GlobalIndex => "global Niles index",
            ArtifactKind::ManagerSession => "manager session metadata",
            ArtifactKind::RunPlan => "run plan",
            ArtifactKind::RunPointer => "run pointer",
            ArtifactKind::RunState => "run state",
            ArtifactKind::StepRecord => "step record",
            ArtifactKind::WorkerMetadata => "worker metadata",
            ArtifactKind::WorkerPointer => "worker pointer",
            ArtifactKind::WorkspaceManifest => "workspace manifest",
        }
    }

    fn remediation(self) -> &'static str {
        match self {
            ArtifactKind::CapabilityManifest => {
                "rerun `niles analyze` to regenerate it, or use the older binary that wrote it"
            }
            ArtifactKind::Directory => "fix the directory permissions and rerun `niles doctor`",
            ArtifactKind::GlobalIndex => {
                "use the binary that wrote it, or remove the index only if you accept losing existing cross-workspace run and worker pointers"
            }
            ArtifactKind::ManagerSession => {
                "remove the session directory and start a fresh manager session, or use the older binary that wrote it"
            }
            ArtifactKind::RunPlan => {
                "remove the run directory and start a new run, or use the older binary that wrote it"
            }
            ArtifactKind::RunPointer => {
                "remove the pointer file so Niles can fall back to a local run directory, or use the older binary that wrote it"
            }
            ArtifactKind::RunState => {
                "remove the run directory and start a new run, or use the older binary to inspect or resume it"
            }
            ArtifactKind::StepRecord => {
                "remove the run directory and start a new run, or use the older binary that wrote it"
            }
            ArtifactKind::WorkerMetadata => {
                "remove the worker dir and respawn, or use the older binary to close it"
            }
            ArtifactKind::WorkerPointer => {
                "remove the pointer file and respawn the worker, or use the older binary that wrote it"
            }
            ArtifactKind::WorkspaceManifest => {
                "delete .niles/manifest.yaml and rerun `niles`, or use the older binary that wrote it"
            }
        }
    }
}

impl SchemaStatus {
    pub(crate) fn is_problem(&self) -> bool {
        !matches!(self, SchemaStatus::Current(_))
    }

    pub(crate) fn summary(&self) -> String {
        match self {
            SchemaStatus::Current(schema) => format!("current schema {schema}"),
            SchemaStatus::Older(schema) => format!("older schema {schema}"),
            SchemaStatus::Newer(schema) => format!("newer schema {schema}"),
            SchemaStatus::Invalid => "invalid schema stamp".to_owned(),
            SchemaStatus::Malformed => "malformed artifact".to_owned(),
            SchemaStatus::Unreadable => "unreadable artifact".to_owned(),
        }
    }
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

fn read_yaml_body<T>(path: &Utf8Path, kind: ArtifactKind, body: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    let value = parse_yaml_value(path, kind, body)?;
    let probe = schema_from_yaml(&value);
    reject_newer_schema(path, kind, probe)?;
    serde_yaml::from_str(body).map_err(|err| deserialize_failure(path, kind, probe, err))
}

fn parse_json_value(path: &Utf8Path, kind: ArtifactKind, body: &str) -> Result<JsonValue> {
    serde_json::from_str(body)
        .map_err(|err| anyhow::Error::new(err).context(malformed_artifact(path, kind, "JSON")))
}

fn parse_yaml_value(path: &Utf8Path, kind: ArtifactKind, body: &str) -> Result<YamlValue> {
    serde_yaml::from_str(body)
        .map_err(|err| anyhow::Error::new(err).context(malformed_artifact(path, kind, "YAML")))
}

fn stamp_json_value(value: &mut JsonValue) -> Result<()> {
    let Some(object) = value.as_object_mut() else {
        bail!("schema-stamped JSON artifacts must serialize as an object");
    };
    object.insert(FIELD.to_owned(), JsonValue::from(CURRENT_SCHEMA));
    Ok(())
}

fn stamp_yaml_value(value: &mut YamlValue) -> Result<()> {
    let YamlValue::Mapping(object) = value else {
        bail!("schema-stamped YAML artifacts must serialize as a mapping");
    };
    object.insert(
        YamlValue::String(FIELD.to_owned()),
        YamlValue::from(CURRENT_SCHEMA),
    );
    Ok(())
}

fn schema_from_json(value: &JsonValue) -> SchemaProbe {
    let Some(object) = value.as_object() else {
        return SchemaProbe::Invalid;
    };
    schema_from_json_object(object)
}

fn schema_from_json_object(object: &JsonMap<String, JsonValue>) -> SchemaProbe {
    match object.get(FIELD) {
        None => SchemaProbe::Schema(LEGACY_SCHEMA),
        Some(value) => value
            .as_u64()
            .map_or(SchemaProbe::Invalid, SchemaProbe::Schema),
    }
}

fn schema_from_yaml(value: &YamlValue) -> SchemaProbe {
    let YamlValue::Mapping(object) = value else {
        return SchemaProbe::Invalid;
    };
    schema_from_yaml_object(object)
}

fn schema_from_yaml_object(object: &YamlMap) -> SchemaProbe {
    let key = YamlValue::String(FIELD.to_owned());
    match object.get(&key) {
        None => SchemaProbe::Schema(LEGACY_SCHEMA),
        Some(value) => value
            .as_u64()
            .map_or(SchemaProbe::Invalid, SchemaProbe::Schema),
    }
}

fn reject_newer_schema(path: &Utf8Path, kind: ArtifactKind, probe: SchemaProbe) -> Result<()> {
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

fn deserialize_failure<E>(
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

fn malformed_artifact(path: &Utf8Path, kind: ArtifactKind, format: &str) -> String {
    format!(
        "{} {path} is malformed {format}; schema is unknown and this binary expects schema {}; {}",
        kind.label(),
        CURRENT_SCHEMA,
        kind.remediation()
    )
}

fn push_json_if_file(
    observations: &mut Vec<SchemaObservation>,
    path: Utf8PathBuf,
    kind: ArtifactKind,
) {
    if path.is_file() {
        observations.push(inspect_json(&path, kind));
    }
}

fn push_yaml_if_file(
    observations: &mut Vec<SchemaObservation>,
    path: Utf8PathBuf,
    kind: ArtifactKind,
) {
    if path.is_file() {
        observations.push(inspect_yaml(&path, kind));
    }
}

fn read_dir_paths(observations: &mut Vec<SchemaObservation>, dir: &Utf8Path) -> Vec<Utf8PathBuf> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Vec::new(),
        Err(_) => {
            observations.push(SchemaObservation {
                kind: ArtifactKind::Directory,
                path: dir.to_path_buf(),
                status: SchemaStatus::Unreadable,
            });
            return Vec::new();
        }
    };

    let mut paths = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            observations.push(SchemaObservation {
                kind: ArtifactKind::Directory,
                path: dir.to_path_buf(),
                status: SchemaStatus::Unreadable,
            });
            continue;
        };
        if let Ok(path) = Utf8PathBuf::from_path_buf(entry.path()) {
            paths.push(path);
        }
    }
    paths.sort();
    paths
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchemaProbe {
    Schema(u64),
    Invalid,
}

impl SchemaProbe {
    fn into_status(self) -> SchemaStatus {
        match self {
            SchemaProbe::Schema(CURRENT_SCHEMA) => SchemaStatus::Current(CURRENT_SCHEMA),
            SchemaProbe::Schema(schema) if schema < CURRENT_SCHEMA => SchemaStatus::Older(schema),
            SchemaProbe::Schema(schema) => SchemaStatus::Newer(schema),
            SchemaProbe::Invalid => SchemaStatus::Invalid,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            read_json::<Example>(&path, ArtifactKind::RunState).unwrap(),
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
            read_json::<Example>(&path, ArtifactKind::RunState).unwrap(),
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

        let err = read_json::<Example>(&path, ArtifactKind::RunState).unwrap_err();
        let err = err.to_string();

        assert!(err.contains("run state"));
        assert!(err.contains("schema 1"));
        assert!(err.contains("expects 2"));
        assert!(err.contains("remove the run directory"));
        assert!(!err.contains("missing field"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn current_schema_deserialization_error_keeps_serde_source() {
        let root = temp_test_path("json-current-corrupt");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("state.json");
        fs::write(&path, r#"{"niles_schema":2,"id":"bad"}"#).unwrap();

        let err = read_json::<Example>(&path, ArtifactKind::RunState).unwrap_err();
        let chain = err.chain().map(|err| err.to_string()).collect::<Vec<_>>();

        assert!(chain[0].contains("declares schema 2"));
        assert!(chain.iter().any(|err| err.contains("missing field")));

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_directories_are_reported_without_aborting_scan() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_test_path("unreadable-dir");
        let steps = root.join(".niles/runs/run-1/steps");
        fs::create_dir_all(&steps).unwrap();
        let mut permissions = fs::metadata(&steps).unwrap().permissions();
        permissions.set_mode(0o000);
        fs::set_permissions(&steps, permissions).unwrap();

        let observations = scan_workspace(&root).unwrap();

        let mut permissions = fs::metadata(&steps).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&steps, permissions).unwrap();
        assert!(observations.iter().any(|observation| {
            observation.kind == ArtifactKind::Directory
                && observation.path == steps
                && observation.status == SchemaStatus::Unreadable
        }));

        fs::remove_dir_all(root).unwrap();
    }

    fn temp_test_path(label: &str) -> Utf8PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
            "niles-schema-{label}-{}-{nanos}",
            std::process::id()
        )))
        .unwrap()
    }
}
