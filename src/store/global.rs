use std::{collections::BTreeMap, fs};

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::{
    schema::{self, ArtifactKind},
    util::{remove_file_if_exists, write_json_pretty},
};

use super::{archive::WorkerArchivePointer, paths::global_index_path, worker::WorkerPointer};

pub(super) fn write_global_worker_pointer(pointer: &WorkerPointer) -> Result<()> {
    let path = global_index_path()?;
    let mut index = read_global_index(&path)?;
    index.workers.insert(pointer.id.clone(), pointer.clone());
    write_global_index(&path, &index)
}

pub(super) fn write_global_worker_archive_pointer(pointer: &WorkerArchivePointer) -> Result<()> {
    let path = global_index_path()?;
    let mut index = read_global_index(&path)?;
    let archives = index.worker_archives.entry(pointer.id.clone()).or_default();
    archives.retain(|archive| archive.archive_dir != pointer.archive_dir);
    archives.push(pointer.clone());
    archives.sort_by(|left, right| {
        left.archived_at
            .cmp(&right.archived_at)
            .then_with(|| left.archive_dir.cmp(&right.archive_dir))
    });
    write_global_index(&path, &index)
}

pub(super) fn remove_global_worker_pointer(worker: &str) -> Result<()> {
    let path = global_index_path()?;
    let mut index = read_global_index(&path)?;
    index.workers.remove(worker);
    if index.workers.is_empty() && index.worker_archives.is_empty() {
        return remove_file_if_exists(&path);
    }
    write_global_index(&path, &index)
}

fn write_global_index(path: &Utf8Path, index: &GlobalIndex) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    }
    write_json_pretty(path, index)
}

pub(super) fn read_global_worker_pointer(worker: &str) -> Result<Option<WorkerPointer>> {
    let path = global_index_path()?;
    Ok(read_global_index(&path)?.workers.get(worker).cloned())
}

#[expect(
    clippy::disallowed_methods,
    reason = "the global index is optional until the first registered worker creates it"
)]
pub(super) fn read_global_index(path: &Utf8Path) -> Result<GlobalIndex> {
    Ok(schema::read_optional_json(path, ArtifactKind::GlobalIndex)?.unwrap_or_default())
}

#[derive(Default, Deserialize, Serialize)]
pub(super) struct GlobalIndex {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) workers: BTreeMap<String, WorkerPointer>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) worker_archives: BTreeMap<String, Vec<WorkerArchivePointer>>,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{read_global_index, write_global_worker_pointer};
    use crate::store::test_support::{create_dir, worker_pointer, ScopedEnv, TempDir};

    #[test]
    fn global_index_uses_niles_home_override() {
        let root = TempDir::new("niles-home-override");
        let niles_home = root.path().join("niles-home");
        let home = root.path().join("home");
        let _env = ScopedEnv::new(&niles_home, &home);
        let niles_home_target = create_dir(root.path().join("niles-home-target"));
        let worker = "shared";

        write_global_worker_pointer(&worker_pointer(worker, &niles_home_target)).unwrap();

        assert_eq!(
            crate::store::global_index_path().unwrap(),
            niles_home.join("runs/index.json")
        );
        let index = read_global_index(&niles_home.join("runs/index.json")).unwrap();
        assert_eq!(index.workers[worker].worker_dir, niles_home_target);
    }

    #[test]
    fn legacy_global_index_reads_and_stamps_on_next_write() {
        let root = TempDir::new("legacy-global-index-stamp");
        let _env = ScopedEnv::new(&root.path().join("niles-home"), &root.path().join("home"));
        let path = crate::store::global_index_path().unwrap();
        let legacy_target = create_dir(root.path().join("legacy-target"));
        let new_target = create_dir(root.path().join("new-target"));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(
            &path,
            format!(
                r#"{{
  "workers": {{
    "legacy": {{
      "id": "legacy",
      "workspace": "{}",
      "worker_dir": "{}",
      "local_stores": []
    }}
  }}
}}
"#,
                root.path(),
                legacy_target
            ),
        )
        .unwrap();

        let index = read_global_index(&path).unwrap();
        assert!(index.workers.contains_key("legacy"));

        write_global_worker_pointer(&worker_pointer("new", &new_target)).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains(r#""niles_schema": 2"#));
        assert!(body.contains(r#""legacy""#));
        assert!(body.contains(r#""new""#));
    }

    #[test]
    fn legacy_global_index_tolerates_removed_runs_key() {
        let root = TempDir::new("legacy-global-index-runs-key");
        let _env = ScopedEnv::new(&root.path().join("niles-home"), &root.path().join("home"));
        let path = crate::store::global_index_path().unwrap();
        let worker_dir = create_dir(root.path().join("worker-target"));
        let run_dir = create_dir(root.path().join("run-target"));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(
            &path,
            format!(
                r#"{{
  "niles_schema": 2,
  "runs": {{
    "legacy-run": {{
      "id": "legacy-run",
      "workspace": "{}",
      "run_dir": "{}"
    }}
  }},
  "workers": {{
    "legacy-worker": {{
      "id": "legacy-worker",
      "workspace": "{}",
      "worker_dir": "{}",
      "local_stores": []
    }}
  }}
}}
"#,
                root.path(),
                run_dir,
                root.path(),
                worker_dir
            ),
        )
        .unwrap();

        let index = read_global_index(&path).unwrap();

        assert_eq!(index.workers.len(), 1);
        assert_eq!(index.workers["legacy-worker"].worker_dir, worker_dir);
    }
}
