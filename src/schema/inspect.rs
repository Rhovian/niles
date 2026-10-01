use std::fs;

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    session::sessions_dir,
    store::{self, ARCHIVE_DIR, paths::NILES_DIR},
    util::read_dir_utf8_paths,
};

use super::{
    kind::ArtifactKind,
    status::{SchemaObservation, SchemaStatus},
    version::{SchemaProbe, schema_from_json},
    yaml::probe_schema,
};

fn inspect(
    path: &Utf8Path,
    kind: ArtifactKind,
    probe: fn(&str) -> Option<SchemaProbe>,
) -> SchemaObservation {
    let status = match fs::read_to_string(path) {
        Ok(body) => probe(&body).map_or(SchemaStatus::Malformed, SchemaProbe::into_status),
        Err(_) => SchemaStatus::Unreadable,
    };
    SchemaObservation {
        kind,
        path: path.to_path_buf(),
        status,
    }
}

fn json_probe(body: &str) -> Option<SchemaProbe> {
    let value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => return None,
    };
    Some(schema_from_json(&value))
}

fn yaml_probe(body: &str) -> Option<SchemaProbe> {
    let probe = match probe_schema(body) {
        Ok(probe) => probe,
        Err(_) => return None,
    };
    Some(probe)
}

pub(crate) fn scan_workspace(root: &Utf8Path) -> Result<Vec<SchemaObservation>> {
    let niles = root.join(NILES_DIR);
    if !niles.exists() {
        return Ok(Vec::new());
    }

    let mut observations = Vec::new();
    push_if_file(
        &mut observations,
        niles.join("manifest.yaml"),
        ArtifactKind::WorkspaceManifest,
        yaml_probe,
    );

    // Archives nest one level deeper and carry no worker `meta.json`.
    let workers = store::workers_dir(root);
    for path in read_dir_paths(&mut observations, &workers) {
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        if name == ARCHIVE_DIR {
            continue;
        }
        push_if_file(
            &mut observations,
            path.join("meta.json"),
            ArtifactKind::WorkerMetadata,
            json_probe,
        );
    }

    let sessions = sessions_dir(root);
    for path in read_dir_paths(&mut observations, &sessions) {
        if path.is_dir() {
            push_if_file(
                &mut observations,
                path.join("session.json"),
                ArtifactKind::ManagerSession,
                json_probe,
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

fn push_if_file(
    observations: &mut Vec<SchemaObservation>,
    path: Utf8PathBuf,
    kind: ArtifactKind,
    probe: fn(&str) -> Option<SchemaProbe>,
) {
    if path.is_file() {
        observations.push(inspect(&path, kind, probe));
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
    use crate::test_support::temp_test_path;

    #[test]
    fn worker_scan_excludes_the_archive_directory() {
        let root = temp_test_path("scan-excludes-archivedir");
        let workers = store::workers_dir(&root);
        fs::create_dir_all(workers.join("worker-1")).unwrap();
        fs::write(workers.join("worker-1/meta.json"), "{}").unwrap();
        // A stray `meta.json` inside `archive` must not be reported as a worker, just as every
        // other reader excludes the archive directory by name.
        fs::create_dir_all(workers.join(ARCHIVE_DIR)).unwrap();
        fs::write(workers.join(ARCHIVE_DIR).join("meta.json"), "{}").unwrap();

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
