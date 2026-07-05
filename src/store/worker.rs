use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::{
    schema::{self, ArtifactKind},
    util::{current_dir_utf8, read_dir_utf8_paths, remove_file_if_exists, write_json_pretty},
};

use super::{
    global::{
        read_global_index, read_global_worker_pointer, remove_global_worker_pointer,
        write_global_worker_pointer,
    },
    paths::{
        NILES_DIR, WORKERS_DIR, current_workers_dir, global_index_path, pointer_file,
        workspace_workers_dir,
    },
};

pub(crate) fn register_worker_location(
    worker: &str,
    workspace: &Utf8Path,
    worker_dir: &Utf8Path,
) -> Result<WorkerPointer> {
    register_worker_location_with_local_store(worker, workspace, worker_dir, current_workers_dir()?)
}

pub(crate) fn unregister_worker_location(
    worker: &str,
    location: Option<&WorkerLocation>,
    workspace: Option<&Utf8Path>,
    worker_dir: Option<&Utf8Path>,
) -> Result<()> {
    let mut stores = BTreeSet::new();
    stores.insert(current_workers_dir()?);

    if let Some(location) = location {
        collect_worker_location_stores(&mut stores, location)?;
    }
    if let Some(pointer) = read_global_worker_pointer(worker)? {
        collect_worker_pointer_stores(&mut stores, &pointer)?;
    }
    if let Some(workspace) = workspace {
        stores.insert(workspace_workers_dir(workspace)?);
    }
    if let Some(worker_dir) = worker_dir
        && let Some(parent) = worker_dir.parent()
    {
        stores.insert(parent.to_path_buf());
    }

    for store in stores {
        remove_file_if_exists(&store.join(pointer_file(worker)))?;
    }
    remove_global_worker_pointer(worker)
}

pub(crate) fn resolve_worker_location(worker: &str) -> Result<Option<WorkerLocation>> {
    WorkerResolver::from_current()?.named(worker)
}

pub(crate) fn resolve_worker_locations() -> Result<Vec<WorkerListEntry>> {
    WorkerResolver::from_current()?.all()
}

pub(crate) fn resolve_workspace_worker_locations() -> Result<Vec<WorkerListEntry>> {
    WorkerResolver::from_current()?.local_directories()
}

pub(super) fn write_local_worker_pointer(
    workers_dir: &Utf8Path,
    pointer: &WorkerPointer,
) -> Result<()> {
    fs::create_dir_all(workers_dir).with_context(|| format!("failed to create {workers_dir}"))?;
    write_json_pretty(&workers_dir.join(pointer_file(&pointer.id)), pointer)
}

pub(super) fn register_worker_location_with_local_store(
    worker: &str,
    workspace: &Utf8Path,
    worker_dir: &Utf8Path,
    local_workers_dir: Utf8PathBuf,
) -> Result<WorkerPointer> {
    let workspace_workers_dir = workspace_workers_dir(workspace)?;
    let mut local_stores = vec![local_workers_dir, workspace_workers_dir];
    local_stores.sort();
    local_stores.dedup();

    let pointer = WorkerPointer {
        id: worker.to_owned(),
        workspace: workspace.to_path_buf(),
        worker_dir: worker_dir.to_path_buf(),
        local_stores,
    };

    for workers_dir in &pointer.local_stores {
        write_local_worker_pointer(workers_dir, &pointer)?;
    }
    write_global_worker_pointer(&pointer)?;
    Ok(pointer)
}

pub(super) struct WorkerResolver {
    pub(super) local_workspace: Utf8PathBuf,
    pub(super) local_workers_dir: Utf8PathBuf,
}

impl WorkerResolver {
    fn from_current() -> Result<Self> {
        let local_workspace = current_dir_utf8()?;
        Ok(Self {
            local_workers_dir: local_workspace.join(NILES_DIR).join(WORKERS_DIR),
            local_workspace,
        })
    }

    pub(super) fn named(&self, worker: &str) -> Result<Option<WorkerLocation>> {
        if let Some(pointer) = self.local_pointer(worker)? {
            return Ok(Some(WorkerLocation::from_pointer(pointer)));
        }

        let local_worker_dir = self.local_workers_dir.join(worker);
        if local_worker_dir.exists() {
            return Ok(Some(WorkerLocation::local(
                self.local_workspace.clone(),
                local_worker_dir,
            )));
        }

        Ok(read_global_worker_pointer(worker)?.map(WorkerLocation::from_pointer))
    }

    fn all(&self) -> Result<Vec<WorkerListEntry>> {
        let mut by_id = BTreeMap::<String, WorkerLocation>::new();
        self.collect_local(&mut by_id)?;

        for pointer in read_global_index(&global_index_path()?)?
            .workers
            .into_values()
        {
            let id = pointer.id.clone();
            by_id
                .entry(id)
                .or_insert_with(|| WorkerLocation::from_pointer(pointer));
        }

        Ok(by_id
            .into_iter()
            .map(|(id, location)| WorkerListEntry { id, location })
            .collect())
    }

    fn local_directories(&self) -> Result<Vec<WorkerListEntry>> {
        let mut workers = read_dir_utf8_paths(&self.local_workers_dir)?
            .into_iter()
            .filter(|path| path.is_dir())
            .filter_map(|path| {
                let id = path.file_name()?.to_owned();
                (id != "archive").then(|| WorkerListEntry {
                    id: id.clone(),
                    location: WorkerLocation::local(self.local_workspace.clone(), path),
                })
            })
            .collect::<Vec<_>>();
        workers.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(workers)
    }

    fn local_pointer(&self, worker: &str) -> Result<Option<WorkerPointer>> {
        read_worker_pointer(&self.local_workers_dir.join(pointer_file(worker)))
    }

    fn collect_local(&self, by_id: &mut BTreeMap<String, WorkerLocation>) -> Result<()> {
        let paths = read_dir_utf8_paths(&self.local_workers_dir)?;

        for path in paths.iter().filter(|path| path.extension() == Some("json")) {
            if let Some(pointer) = read_worker_pointer(path)? {
                let id = pointer.id.clone();
                by_id
                    .entry(id)
                    .or_insert_with(|| WorkerLocation::from_pointer(pointer));
            }
        }

        for path in paths.iter().filter(|path| path.is_dir()) {
            let Some(id) = path.file_name() else {
                continue;
            };
            if id == "archive" {
                continue;
            }
            by_id.entry(id.to_owned()).or_insert_with(|| {
                WorkerLocation::local(self.local_workspace.clone(), path.to_path_buf())
            });
        }

        Ok(())
    }
}

pub(super) fn read_worker_pointer(path: &Utf8Path) -> Result<Option<WorkerPointer>> {
    schema::read_optional_json(path, ArtifactKind::WorkerPointer)
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct WorkerPointer {
    pub(crate) id: String,
    pub(crate) workspace: Utf8PathBuf,
    pub(crate) worker_dir: Utf8PathBuf,
    #[serde(default)]
    pub(crate) local_stores: Vec<Utf8PathBuf>,
}

#[derive(Clone)]
pub(crate) struct WorkerLocation {
    pub(crate) workspace: Utf8PathBuf,
    pub(crate) worker_dir: Utf8PathBuf,
    pointer: Option<WorkerPointer>,
}

#[derive(Clone)]
pub(crate) struct WorkerListEntry {
    pub(crate) id: String,
    pub(crate) location: WorkerLocation,
}

impl WorkerLocation {
    fn from_pointer(pointer: WorkerPointer) -> Self {
        Self {
            workspace: pointer.workspace.clone(),
            worker_dir: pointer.worker_dir.clone(),
            pointer: Some(pointer),
        }
    }

    fn local(workspace: Utf8PathBuf, worker_dir: Utf8PathBuf) -> Self {
        Self {
            workspace,
            worker_dir,
            pointer: None,
        }
    }
}

fn collect_worker_location_stores(
    stores: &mut BTreeSet<Utf8PathBuf>,
    location: &WorkerLocation,
) -> Result<()> {
    stores.insert(workspace_workers_dir(&location.workspace)?);
    if let Some(parent) = location.worker_dir.parent() {
        stores.insert(parent.to_path_buf());
    }
    if let Some(pointer) = &location.pointer {
        collect_worker_pointer_stores(stores, pointer)?;
    }
    Ok(())
}

fn collect_worker_pointer_stores(
    stores: &mut BTreeSet<Utf8PathBuf>,
    pointer: &WorkerPointer,
) -> Result<()> {
    stores.insert(workspace_workers_dir(&pointer.workspace)?);
    if let Some(parent) = pointer.worker_dir.parent() {
        stores.insert(parent.to_path_buf());
    }
    stores.extend(pointer.local_stores.iter().cloned());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::store::{
        paths::pointer_file,
        test_support::{
            ScopedEnv, TempDir, assert_worker_resolves_to, create_dir, worker_pointer,
            worker_resolver_at, write_global_worker_pointer,
        },
        worker::{register_worker_location_with_local_store, write_local_worker_pointer},
    };

    #[test]
    fn registered_worker_resolves_from_invoking_project_and_unrelated_stores() {
        let root = TempDir::new("worker-registration-resolution");
        let _env = ScopedEnv::new(&root.path().join("niles-home"), &root.path().join("home"));
        let invoker = create_dir(root.path().join("invoker"));
        let project = create_dir(root.path().join("project"));
        let unrelated = create_dir(root.path().join("unrelated"));
        let worker = "auth-fix";
        let invoker_workers_dir = invoker.join(".niles/worker");
        let project_workers_dir = project.join(".niles/worker");
        let unrelated_workers_dir = unrelated.join(".niles/worker");
        let worker_dir = create_dir(project_workers_dir.join(worker));

        register_worker_location_with_local_store(
            worker,
            &project,
            &worker_dir,
            invoker_workers_dir.clone(),
        )
        .unwrap();

        assert!(invoker_workers_dir.join(pointer_file(worker)).is_file());
        assert!(project_workers_dir.join(pointer_file(worker)).is_file());
        assert_worker_resolves_to(&invoker_workers_dir, worker, &worker_dir);
        assert_worker_resolves_to(&project_workers_dir, worker, &worker_dir);
        assert_worker_resolves_to(&unrelated_workers_dir, worker, &worker_dir);
    }

    #[test]
    fn named_worker_resolution_prefers_local_pointer_then_local_dir_then_global() {
        let root = TempDir::new("worker-resolution-chain");
        let _env = ScopedEnv::new(&root.path().join("niles-home"), &root.path().join("home"));
        let local_workers_dir = root.path().join("workspace/.niles/worker");
        fs::create_dir_all(&local_workers_dir).unwrap();

        let worker = "auth-fix";
        let local_pointer_target = create_dir(root.path().join("pointer-target"));
        let local_worker_dir = create_dir(local_workers_dir.join(worker));
        let global_target = create_dir(root.path().join("global-target"));
        write_local_worker_pointer(
            &local_workers_dir,
            &worker_pointer(worker, &local_pointer_target),
        )
        .unwrap();
        write_global_worker_pointer(&worker_pointer(worker, &global_target)).unwrap();

        let resolver = worker_resolver_at(&local_workers_dir);
        assert_eq!(
            resolver.named(worker).unwrap().unwrap().worker_dir,
            local_pointer_target
        );

        fs::remove_file(local_workers_dir.join(pointer_file(worker))).unwrap();
        assert_eq!(
            resolver.named(worker).unwrap().unwrap().worker_dir,
            local_worker_dir
        );

        fs::remove_dir_all(&local_worker_dir).unwrap();
        assert_eq!(
            resolver.named(worker).unwrap().unwrap().worker_dir,
            global_target
        );

        fs::remove_file(crate::store::global_index_path().unwrap()).unwrap();
        assert!(resolver.named(worker).unwrap().is_none());
    }
}
