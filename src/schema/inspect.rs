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
use crate::store::paths::{NILES_DIR, WORKERS_DIR};

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
    let niles = root.join(NILES_DIR);
    if !niles.exists() {
        return Ok(Vec::new());
    }

    let mut observations = Vec::new();
    push_yaml_if_file(
        &mut observations,
        niles.join("manifest.yaml"),
        ArtifactKind::WorkspaceManifest,
    );

    // The worker layout is owned by `store`; route the scan through the same reader so a rename of
    // `WORKERS_DIR` reaches doctor, and so the `archive` directory is excluded here exactly as it
    // is everywhere else (it nests one level deeper and carries no `meta.json`).
    let workers = niles.join(WORKERS_DIR);
    for path in read_dir_paths(&mut observations, &workers) {
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        if name == "archive" {
            continue;
        }
        push_json_if_file(
            &mut observations,
            path.join("meta.json"),
            ArtifactKind::WorkerMetadata,
        );
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
        observations.push(inspect_yaml(&path, kind));
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
    fn worker_scan_excludes_the_archive_directory() {
        let root = temp_test_path("scan-excludes-archivedir");
        let workers = root.join(NILES_DIR).join(WORKERS_DIR);
        fs::create_dir_all(workers.join("worker-1")).unwrap();
        fs::write(workers.join("worker-1/meta.json"), "{}").unwrap();
        // A stray `meta.json` inside `archive` must not be reported as a worker, just as every
        // other reader excludes the archive directory by name.
        fs::create_dir_all(workers.join("archive")).unwrap();
        fs::write(workers.join("archive/meta.json"), "{}").unwrap();

        let observations = scan_workspace(&root).unwrap();

        let worker_obs: Vec<_> = observations
            .iter()
            .filter(|o| o.kind == ArtifactKind::WorkerMetadata)
            .collect();
        assert_eq!(worker_obs.len(), 1, "{observations:?}");
        assert!(
            worker_obs[0].path.as_str().ends_with("worker-1/meta.json"),
            "{observations:?}"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_directories_are_reported_without_aborting_scan() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_test_path("unreadable-dir");
        let workers = root.join(".niles/worker");
        fs::create_dir_all(&workers).unwrap();
        let mut permissions = fs::metadata(&workers).unwrap().permissions();
        permissions.set_mode(0o000);
        fs::set_permissions(&workers, permissions).unwrap();

        let observations = scan_workspace(&root).unwrap();

        let mut permissions = fs::metadata(&workers).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&workers, permissions).unwrap();
        assert!(observations.iter().any(|observation| {
            observation.kind == ArtifactKind::Directory
                && observation.path == workers
                && observation.status == SchemaStatus::Unreadable
        }));

        fs::remove_dir_all(root).unwrap();
    }
}
