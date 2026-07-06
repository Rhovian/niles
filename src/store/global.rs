use std::{collections::BTreeMap, fs};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::{
    schema::{self, ArtifactKind},
    util::{remove_file_if_exists, write_json_pretty},
};

use super::{
    archive::WorkerArchivePointer, paths::global_index_path, run::RunPointer, worker::WorkerPointer,
};

pub(super) fn write_global_run_pointer(pointer: &RunPointer) -> Result<()> {
    let path = global_index_path()?;
    let mut index = read_global_index(&path)?;
    index.runs.insert(pointer.id.clone(), pointer.clone());
    write_global_index(&path, &index)
}

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
    if index.runs.is_empty() && index.workers.is_empty() && index.worker_archives.is_empty() {
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

pub(super) fn resolve_global_run(run: &str) -> Result<Option<Utf8PathBuf>> {
    let path = global_index_path()?;
    Ok(read_global_index(&path)?
        .runs
        .get(run)
        .map(|pointer| pointer.run_dir.clone()))
}

pub(super) fn read_global_worker_pointer(worker: &str) -> Result<Option<WorkerPointer>> {
    let path = global_index_path()?;
    Ok(read_global_index(&path)?.workers.get(worker).cloned())
}

#[expect(
    clippy::disallowed_methods,
    reason = "missing global archive entries mean this worker has no global archives; local archive discovery still runs"
)]
pub(super) fn global_worker_archives(worker: &str) -> Result<Vec<WorkerArchivePointer>> {
    let path = global_index_path()?;
    Ok(read_global_index(&path)?
        .worker_archives
        .get(worker)
        .cloned()
        .unwrap_or_default())
}

pub(super) fn latest_global_run_dir() -> Result<Option<Utf8PathBuf>> {
    let path = global_index_path()?;
    Ok(read_global_index(&path)?
        .runs
        .into_values()
        .rev()
        .map(|pointer| pointer.run_dir)
        .find(|run_dir| run_dir.is_dir()))
}

#[expect(
    clippy::disallowed_methods,
    reason = "the global index is optional until the first registered run or worker creates it"
)]
pub(super) fn read_global_index(path: &Utf8Path) -> Result<GlobalIndex> {
    Ok(schema::read_optional_json(path, ArtifactKind::GlobalIndex)?.unwrap_or_default())
}

#[derive(Default, Deserialize, Serialize)]
pub(super) struct GlobalIndex {
    #[serde(default)]
    pub(super) runs: BTreeMap<String, RunPointer>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) workers: BTreeMap<String, WorkerPointer>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) worker_archives: BTreeMap<String, Vec<WorkerArchivePointer>>,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{read_global_index, write_global_run_pointer};
    use crate::store::test_support::{
        ScopedEnv, TempDir, create_dir, resolver_at, run_pointer, write_index,
    };

    #[test]
    fn global_index_uses_niles_home_override() {
        let root = TempDir::new("niles-home-override");
        let niles_home = root.path().join("niles-home");
        let home = root.path().join("home");
        let _env = ScopedEnv::new(&niles_home, &home);
        let local_runs_dir = root.path().join("workspace/.niles/runs");
        let niles_home_target = create_dir(root.path().join("niles-home-target"));
        let home_target = create_dir(root.path().join("home-target"));
        let run = "shared";

        write_index(
            &niles_home.join("runs/index.json"),
            &[run_pointer(run, &niles_home_target)],
        );
        write_index(
            &home.join(".niles/runs/index.json"),
            &[run_pointer(run, &home_target)],
        );

        assert_eq!(
            crate::store::global_index_path().unwrap(),
            niles_home.join("runs/index.json")
        );
        assert_eq!(
            resolver_at(&local_runs_dir).named(run).unwrap(),
            niles_home_target
        );
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
  "runs": {{
    "legacy": {{
      "id": "legacy",
      "workspace": "{}",
      "run_dir": "{}"
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
        assert!(index.runs.contains_key("legacy"));

        write_global_run_pointer(&run_pointer("new", &new_target)).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains(r#""niles_schema": 2"#));
        assert!(body.contains(r#""legacy""#));
        assert!(body.contains(r#""new""#));
    }
}
