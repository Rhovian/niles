use std::fs;

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;

use crate::util::read_dir_utf8_paths;

use super::{
    kind::ArtifactKind,
    status::{SchemaObservation, SchemaStatus},
    version::{schema_from_json, schema_from_yaml},
};

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
        observations.push(super::inspect_yaml(&path, kind));
    }
}

fn read_dir_paths(observations: &mut Vec<SchemaObservation>, dir: &Utf8Path) -> Vec<Utf8PathBuf> {
    match read_dir_utf8_paths(dir) {
        Ok(paths) => paths,
        Err(_) => {
            observations.push(SchemaObservation {
                kind: ArtifactKind::Directory,
                path: dir.to_path_buf(),
                status: SchemaStatus::Unreadable,
            });
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::test_support::temp_test_path;

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
}
