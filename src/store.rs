use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::PathBuf,
};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    schema::{self, ArtifactKind},
    state::{RunState, StepRecord},
    util::{
        absolute_path, current_dir_utf8, read_dir_utf8_paths, remove_file_if_exists, utf8_path,
        write_json_pretty,
    },
};

const NILES_DIR: &str = ".niles";
const RUNS_DIR: &str = "runs";
const WORKERS_DIR: &str = "worker";
const LATEST_POINTER: &str = "latest.json";

pub fn write_state(path: &Utf8Path, state: &RunState) -> Result<()> {
    write_json_pretty(path, state)
}

pub fn read_state(run_dir: &Utf8Path) -> Result<RunState> {
    let path = state_path(run_dir);
    schema::read_json(&path, ArtifactKind::RunState)
}

pub fn state_path(run_dir: &Utf8Path) -> Utf8PathBuf {
    run_dir.join("state.json")
}

pub fn workspace_runs_dir(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    Ok(absolute_path(workspace)?.join(NILES_DIR).join(RUNS_DIR))
}

pub fn workspace_run_dir(workspace: &Utf8Path, run: &str) -> Result<Utf8PathBuf> {
    Ok(workspace_runs_dir(workspace)?.join(run))
}

pub(crate) fn workspace_workers_dir(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    Ok(absolute_path(workspace)?.join(NILES_DIR).join(WORKERS_DIR))
}

pub(crate) fn workspace_worker_dir(workspace: &Utf8Path, worker: &str) -> Result<Utf8PathBuf> {
    Ok(workspace_workers_dir(workspace)?.join(worker))
}

pub fn register_run_location(run: &str, workspace: &Utf8Path, run_dir: &Utf8Path) -> Result<()> {
    let pointer = RunPointer {
        id: run.to_owned(),
        workspace: workspace.to_path_buf(),
        run_dir: run_dir.to_path_buf(),
    };

    write_local_run_pointer(&current_runs_dir()?, &pointer)?;
    write_local_run_pointer(&workspace.join(NILES_DIR).join(RUNS_DIR), &pointer)?;
    write_global_run_pointer(&pointer)
}

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

pub(crate) fn register_worker_archive(
    worker: &str,
    archive_dir: &Utf8Path,
    archived_at: DateTime<Utc>,
) -> Result<WorkerArchivePointer> {
    let pointer = WorkerArchivePointer {
        id: worker.to_owned(),
        archive_dir: archive_dir.to_path_buf(),
        archived_at,
    };
    write_global_worker_archive_pointer(&pointer)?;
    Ok(pointer)
}

pub(crate) fn resolve_worker_archives(worker: &str) -> Result<Vec<WorkerArchivePointer>> {
    let mut archives = BTreeMap::new();
    for archive in local_worker_archives(worker)? {
        archives.insert(archive.archive_dir.clone(), archive);
    }
    for archive in global_worker_archives(worker)? {
        archives.insert(archive.archive_dir.clone(), archive);
    }

    let mut archives = archives.into_values().collect::<Vec<_>>();
    archives.sort_by(|left, right| {
        left.archived_at
            .cmp(&right.archived_at)
            .then_with(|| left.archive_dir.cmp(&right.archive_dir))
    });
    Ok(archives)
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

pub fn resolve_run_dir(run: &str) -> Result<Utf8PathBuf> {
    let resolver = RunResolver::from_current()?;
    if run == "latest" {
        return resolver.latest()?.context("no runs found");
    }

    resolver.named(run)
}

pub(crate) fn resolve_latest_run_dir_from(workspace: &Utf8Path) -> Result<Option<Utf8PathBuf>> {
    RunResolver {
        local_runs_dir: workspace_runs_dir(workspace)?,
    }
    .latest()
}

fn current_runs_dir() -> Result<Utf8PathBuf> {
    Ok(current_dir_utf8()?.join(NILES_DIR).join(RUNS_DIR))
}

fn current_workers_dir() -> Result<Utf8PathBuf> {
    Ok(current_dir_utf8()?.join(NILES_DIR).join(WORKERS_DIR))
}

fn worker_archive_root(workers_dir: &Utf8Path) -> Utf8PathBuf {
    workers_dir.join("archive")
}

fn write_local_run_pointer(runs_dir: &Utf8Path, pointer: &RunPointer) -> Result<()> {
    fs::create_dir_all(runs_dir).with_context(|| format!("failed to create {runs_dir}"))?;
    write_run_pointer(runs_dir, RunPointerFile::Run(&pointer.id), pointer)?;
    write_run_pointer(runs_dir, RunPointerFile::Latest, pointer)
}

fn write_local_worker_pointer(workers_dir: &Utf8Path, pointer: &WorkerPointer) -> Result<()> {
    fs::create_dir_all(workers_dir).with_context(|| format!("failed to create {workers_dir}"))?;
    write_json_pretty(&workers_dir.join(pointer_file(&pointer.id)), pointer)
}

fn register_worker_location_with_local_store(
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

struct RunResolver {
    local_runs_dir: Utf8PathBuf,
}

impl RunResolver {
    fn from_current() -> Result<Self> {
        Ok(Self {
            local_runs_dir: current_runs_dir()?,
        })
    }

    fn named(&self, run: &str) -> Result<Utf8PathBuf> {
        if let Some(run_dir) = self.local_pointer(RunPointerFile::Run(run))? {
            return Ok(run_dir);
        }

        let local_run_dir = self.local_runs_dir.join(run);
        if local_run_dir.exists() {
            return Ok(local_run_dir);
        }

        if let Some(run_dir) = resolve_global_run(run)? {
            return Ok(run_dir);
        }

        Ok(local_run_dir)
    }

    // Pointers can outlive their run dirs when `.niles/runs` is cleaned
    // externally; latest means the newest run that still exists.
    fn latest(&self) -> Result<Option<Utf8PathBuf>> {
        if let Some(run_dir) = self
            .local_pointer(RunPointerFile::Latest)?
            .filter(|run_dir| run_dir.is_dir())
        {
            return Ok(Some(run_dir));
        }
        if let Some(run_dir) = latest_local_run_dir(&self.local_runs_dir)? {
            return Ok(Some(run_dir));
        }
        latest_global_run_dir()
    }

    fn local_pointer(&self, pointer_file: RunPointerFile<'_>) -> Result<Option<Utf8PathBuf>> {
        read_pointer(&pointer_file.path(&self.local_runs_dir))
            .map(|pointer| pointer.map(|pointer| pointer.run_dir))
    }
}

struct WorkerResolver {
    local_workers_dir: Utf8PathBuf,
}

impl WorkerResolver {
    fn from_current() -> Result<Self> {
        Ok(Self {
            local_workers_dir: current_workers_dir()?,
        })
    }

    fn named(&self, worker: &str) -> Result<Option<WorkerLocation>> {
        if let Some(pointer) = self.local_pointer(worker)? {
            return Ok(Some(WorkerLocation::from_pointer(pointer)));
        }

        let local_worker_dir = self.local_workers_dir.join(worker);
        if local_worker_dir.exists() {
            return Ok(Some(WorkerLocation::local(local_worker_dir)));
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
                    location: WorkerLocation::local(path),
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
            by_id
                .entry(id.to_owned())
                .or_insert_with(|| WorkerLocation::local(path.to_path_buf()));
        }

        Ok(())
    }
}

#[derive(Clone, Copy)]
enum RunPointerFile<'a> {
    Run(&'a str),
    Latest,
}

impl RunPointerFile<'_> {
    fn path(self, runs_dir: &Utf8Path) -> Utf8PathBuf {
        match self {
            RunPointerFile::Run(run) => runs_dir.join(pointer_file(run)),
            RunPointerFile::Latest => runs_dir.join(LATEST_POINTER),
        }
    }
}

fn write_run_pointer(
    runs_dir: &Utf8Path,
    pointer_file: RunPointerFile<'_>,
    pointer: &RunPointer,
) -> Result<()> {
    write_json_pretty(&pointer_file.path(runs_dir), pointer)
}

fn latest_local_run_dir(runs_dir: &Utf8Path) -> Result<Option<Utf8PathBuf>> {
    let mut runs = read_dir_utf8_paths(runs_dir)?
        .into_iter()
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();

    Ok(runs.pop())
}

fn read_pointer(path: &Utf8Path) -> Result<Option<RunPointer>> {
    schema::read_optional_json(path, ArtifactKind::RunPointer)
}

fn read_worker_pointer(path: &Utf8Path) -> Result<Option<WorkerPointer>> {
    schema::read_optional_json(path, ArtifactKind::WorkerPointer)
}

fn write_global_run_pointer(pointer: &RunPointer) -> Result<()> {
    let path = global_index_path()?;
    let mut index = read_global_index(&path)?;
    index.runs.insert(pointer.id.clone(), pointer.clone());
    write_global_index(&path, &index)
}

fn write_global_worker_pointer(pointer: &WorkerPointer) -> Result<()> {
    let path = global_index_path()?;
    let mut index = read_global_index(&path)?;
    index.workers.insert(pointer.id.clone(), pointer.clone());
    write_global_index(&path, &index)
}

fn write_global_worker_archive_pointer(pointer: &WorkerArchivePointer) -> Result<()> {
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

fn remove_global_worker_pointer(worker: &str) -> Result<()> {
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

fn resolve_global_run(run: &str) -> Result<Option<Utf8PathBuf>> {
    let path = global_index_path()?;
    Ok(read_global_index(&path)?
        .runs
        .get(run)
        .map(|pointer| pointer.run_dir.clone()))
}

fn read_global_worker_pointer(worker: &str) -> Result<Option<WorkerPointer>> {
    let path = global_index_path()?;
    Ok(read_global_index(&path)?.workers.get(worker).cloned())
}

fn global_worker_archives(worker: &str) -> Result<Vec<WorkerArchivePointer>> {
    let path = global_index_path()?;
    Ok(read_global_index(&path)?
        .worker_archives
        .get(worker)
        .cloned()
        .unwrap_or_default())
}

fn latest_global_run_dir() -> Result<Option<Utf8PathBuf>> {
    let path = global_index_path()?;
    Ok(read_global_index(&path)?
        .runs
        .into_values()
        .rev()
        .map(|pointer| pointer.run_dir)
        .find(|run_dir| run_dir.is_dir()))
}

fn read_global_index(path: &Utf8Path) -> Result<GlobalIndex> {
    Ok(schema::read_optional_json(path, ArtifactKind::GlobalIndex)?.unwrap_or_default())
}

pub(crate) fn global_index_path() -> Result<Utf8PathBuf> {
    if let Some(home) = env::var_os("NILES_HOME") {
        return Ok(utf8_path(PathBuf::from(home), "NILES_HOME")?
            .join(RUNS_DIR)
            .join("index.json"));
    }

    let home = env::var_os("HOME").context("HOME is not set; cannot write Niles run index")?;
    Ok(utf8_path(PathBuf::from(home), "HOME")?
        .join(NILES_DIR)
        .join(RUNS_DIR)
        .join("index.json"))
}

fn pointer_file(run: &str) -> String {
    format!("{run}.json")
}

pub fn selected_step(state: &RunState, step: Option<usize>) -> Result<&StepRecord> {
    match step {
        Some(step) => state
            .steps
            .iter()
            .find(|record| record.index == step)
            .with_context(|| format!("step {step} not found")),
        None => state.steps.last().context("run has no recorded steps"),
    }
}

#[derive(Clone, Deserialize, Serialize)]
struct RunPointer {
    id: String,
    workspace: Utf8PathBuf,
    run_dir: Utf8PathBuf,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct WorkerPointer {
    pub(crate) id: String,
    pub(crate) workspace: Utf8PathBuf,
    pub(crate) worker_dir: Utf8PathBuf,
    #[serde(default)]
    pub(crate) local_stores: Vec<Utf8PathBuf>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct WorkerArchivePointer {
    pub(crate) id: String,
    pub(crate) archive_dir: Utf8PathBuf,
    pub(crate) archived_at: DateTime<Utc>,
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

    fn local(worker_dir: Utf8PathBuf) -> Self {
        let workspace = worker_dir
            .parent()
            .and_then(Utf8Path::parent)
            .and_then(Utf8Path::parent)
            .unwrap_or(Utf8Path::new("."))
            .to_path_buf();
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

#[derive(Default, Deserialize, Serialize)]
struct GlobalIndex {
    #[serde(default)]
    runs: BTreeMap<String, RunPointer>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    workers: BTreeMap<String, WorkerPointer>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    worker_archives: BTreeMap<String, Vec<WorkerArchivePointer>>,
}

fn local_worker_archives(worker: &str) -> Result<Vec<WorkerArchivePointer>> {
    let archive_root = worker_archive_root(&current_workers_dir()?);
    let mut archives = Vec::new();
    for path in read_dir_utf8_paths(&archive_root)? {
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        let Some(archived_at) = worker_archive_timestamp(worker, name) else {
            continue;
        };
        archives.push(WorkerArchivePointer {
            id: worker.to_owned(),
            archive_dir: path,
            archived_at,
        });
    }
    Ok(archives)
}

#[expect(
    clippy::disallowed_methods,
    reason = "timestamp parse failure can be a prefix-sibling worker archive, so archive discovery must skip it"
)]
fn worker_archive_timestamp(worker: &str, archive_name: &str) -> Option<DateTime<Utc>> {
    let timestamp = archive_name.strip_prefix(&format!("{worker}-"))?;
    let timestamp = NaiveDateTime::parse_from_str(timestamp, "%Y%m%dT%H%M%S%fZ").ok()?;
    Some(DateTime::from_naive_utc_and_offset(timestamp, Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{
        env,
        ffi::OsString,
        sync::{Mutex, MutexGuard},
        time::{SystemTime, UNIX_EPOCH},
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn named_run_resolution_prefers_local_pointer_then_local_dir_then_global_then_fallback() {
        let root = TempDir::new("named-resolution-chain");
        let _env = ScopedEnv::new(&root.path().join("niles-home"), &root.path().join("home"));
        let local_runs_dir = root.path().join("workspace/.niles/runs");
        fs::create_dir_all(&local_runs_dir).unwrap();

        let run = "build";
        let local_pointer_target = create_dir(root.path().join("pointer-target"));
        let local_run_dir = create_dir(local_runs_dir.join(run));
        let global_target = create_dir(root.path().join("global-target"));
        write_local_run_pointer(&local_runs_dir, run, &local_pointer_target);
        write_global_run_pointer(run, &global_target);

        let resolver = resolver_at(&local_runs_dir);
        assert_eq!(resolver.named(run).unwrap(), local_pointer_target);

        fs::remove_file(local_runs_dir.join(pointer_file(run))).unwrap();
        assert_eq!(resolver.named(run).unwrap(), local_run_dir);

        fs::remove_dir_all(&local_run_dir).unwrap();
        assert_eq!(resolver.named(run).unwrap(), global_target);

        fs::remove_file(global_index_path().unwrap()).unwrap();
        assert_eq!(resolver.named(run).unwrap(), local_run_dir);
    }

    #[test]
    fn latest_resolution_prefers_latest_pointer_then_latest_local_dir_then_latest_global() {
        let root = TempDir::new("latest-resolution-chain");
        let _env = ScopedEnv::new(&root.path().join("niles-home"), &root.path().join("home"));
        let local_runs_dir = root.path().join("workspace/.niles/runs");
        fs::create_dir_all(&local_runs_dir).unwrap();

        let latest_pointer_target = create_dir(root.path().join("pointer-latest"));
        let older_local = create_dir(local_runs_dir.join("2024-01-01"));
        let newer_local = create_dir(local_runs_dir.join("2024-02-01"));
        let older_global = create_dir(root.path().join("global-older"));
        let newer_global = create_dir(root.path().join("global-newer"));
        write_latest_pointer(&local_runs_dir, "pointer-latest", &latest_pointer_target);
        write_global_run_pointer("2024-01-01", &older_global);
        write_global_run_pointer("2024-03-01", &newer_global);

        let resolver = resolver_at(&local_runs_dir);
        assert_eq!(resolver.latest().unwrap(), Some(latest_pointer_target));

        fs::remove_file(local_runs_dir.join(LATEST_POINTER)).unwrap();
        assert_eq!(resolver.latest().unwrap(), Some(newer_local));

        fs::remove_dir_all(&older_local).unwrap();
        fs::remove_dir_all(local_runs_dir.join("2024-02-01")).unwrap();
        assert_eq!(resolver.latest().unwrap(), Some(newer_global));
    }

    #[test]
    fn latest_resolution_skips_pointers_to_deleted_run_dirs() {
        let root = TempDir::new("latest-dangling-pointers");
        let _env = ScopedEnv::new(&root.path().join("niles-home"), &root.path().join("home"));
        let local_runs_dir = root.path().join("workspace/.niles/runs");
        fs::create_dir_all(&local_runs_dir).unwrap();

        write_latest_pointer(
            &local_runs_dir,
            "deleted-local",
            &root.path().join("deleted-pointer-target"),
        );
        let existing_global = create_dir(root.path().join("global-existing"));
        write_global_run_pointer("2024-01-01", &existing_global);
        write_global_run_pointer("2024-02-01", &root.path().join("deleted-global-target"));

        let resolver = resolver_at(&local_runs_dir);
        assert_eq!(resolver.latest().unwrap(), Some(existing_global));

        fs::remove_file(global_index_path().unwrap()).unwrap();
        assert_eq!(resolver.latest().unwrap(), None);
    }

    #[test]
    fn corrupt_pointer_json_returns_parse_error() {
        let root = TempDir::new("corrupt-pointer");
        let _env = ScopedEnv::new(&root.path().join("niles-home"), &root.path().join("home"));
        let local_runs_dir = root.path().join("workspace/.niles/runs");
        fs::create_dir_all(&local_runs_dir).unwrap();
        fs::write(local_runs_dir.join(pointer_file("bad")), "{").unwrap();
        fs::write(local_runs_dir.join(LATEST_POINTER), "{").unwrap();

        let resolver = resolver_at(&local_runs_dir);
        let named_error = resolver.named("bad").unwrap_err().to_string();
        let latest_error = resolver.latest().unwrap_err().to_string();

        assert!(named_error.contains("run pointer"));
        assert!(named_error.contains("malformed JSON"));
        assert!(named_error.contains("schema is unknown"));
        assert!(latest_error.contains("run pointer"));
        assert!(latest_error.contains("malformed JSON"));
        assert!(latest_error.contains("schema is unknown"));
    }

    #[test]
    fn missing_and_empty_runs_dirs_resolve_to_none_or_named_fallback_path() {
        let root = TempDir::new("missing-empty-runs");
        let _env = ScopedEnv::new(&root.path().join("niles-home"), &root.path().join("home"));
        let local_runs_dir = root.path().join("workspace/.niles/runs");
        let resolver = resolver_at(&local_runs_dir);

        assert_eq!(resolver.latest().unwrap(), None);
        assert_eq!(
            resolver.named("missing").unwrap(),
            local_runs_dir.join("missing")
        );

        fs::create_dir_all(&local_runs_dir).unwrap();
        assert_eq!(resolver.latest().unwrap(), None);
        assert_eq!(
            resolver.named("empty").unwrap(),
            local_runs_dir.join("empty")
        );
    }

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
            global_index_path().unwrap(),
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
        let path = global_index_path().unwrap();
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

        super::write_global_run_pointer(&run_pointer("new", &new_target)).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains(r#""niles_schema": 2"#));
        assert!(body.contains(r#""legacy""#));
        assert!(body.contains(r#""new""#));
    }

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

        fs::remove_file(global_index_path().unwrap()).unwrap();
        assert!(resolver.named(worker).unwrap().is_none());
    }

    #[test]
    fn stale_local_pointers_resolve_named_but_not_latest() {
        let root = TempDir::new("stale-pointer");
        let _env = ScopedEnv::new(&root.path().join("niles-home"), &root.path().join("home"));
        let local_runs_dir = root.path().join("workspace/.niles/runs");
        fs::create_dir_all(&local_runs_dir).unwrap();
        let stale_target = root.path().join("removed-run-dir");

        write_local_run_pointer(&local_runs_dir, "stale", &stale_target);
        write_latest_pointer(&local_runs_dir, "stale", &stale_target);

        let resolver = resolver_at(&local_runs_dir);
        assert!(!stale_target.exists());
        assert_eq!(resolver.named("stale").unwrap(), stale_target);
        assert_eq!(resolver.latest().unwrap(), None);
    }

    fn resolver_at(local_runs_dir: &Utf8Path) -> RunResolver {
        RunResolver {
            local_runs_dir: local_runs_dir.to_path_buf(),
        }
    }

    fn worker_resolver_at(local_workers_dir: &Utf8Path) -> WorkerResolver {
        WorkerResolver {
            local_workers_dir: local_workers_dir.to_path_buf(),
        }
    }

    fn assert_worker_resolves_to(
        local_workers_dir: &Utf8Path,
        worker: &str,
        worker_dir: &Utf8Path,
    ) {
        assert_eq!(
            worker_resolver_at(local_workers_dir)
                .named(worker)
                .unwrap()
                .unwrap()
                .worker_dir,
            worker_dir
        );
    }

    fn create_dir(path: Utf8PathBuf) -> Utf8PathBuf {
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_local_run_pointer(runs_dir: &Utf8Path, run: &str, run_dir: &Utf8Path) {
        write_run_pointer(
            runs_dir,
            RunPointerFile::Run(run),
            &run_pointer(run, run_dir),
        )
        .unwrap();
    }

    fn write_latest_pointer(runs_dir: &Utf8Path, run: &str, run_dir: &Utf8Path) {
        write_run_pointer(runs_dir, RunPointerFile::Latest, &run_pointer(run, run_dir)).unwrap();
    }

    fn write_global_run_pointer(run: &str, run_dir: &Utf8Path) {
        super::write_global_run_pointer(&run_pointer(run, run_dir)).unwrap();
    }

    fn write_index(path: &Utf8Path, pointers: &[RunPointer]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }

        let mut index = GlobalIndex::default();
        for pointer in pointers {
            index.runs.insert(pointer.id.clone(), pointer.clone());
        }
        write_json_pretty(path, &index).unwrap();
    }

    fn run_pointer(run: &str, run_dir: &Utf8Path) -> RunPointer {
        RunPointer {
            id: run.to_owned(),
            workspace: run_dir.parent().unwrap_or(Utf8Path::new("/")).to_path_buf(),
            run_dir: run_dir.to_path_buf(),
        }
    }

    fn worker_pointer(worker: &str, worker_dir: &Utf8Path) -> WorkerPointer {
        WorkerPointer {
            id: worker.to_owned(),
            workspace: worker_dir
                .parent()
                .and_then(Utf8Path::parent)
                .and_then(Utf8Path::parent)
                .unwrap_or(Utf8Path::new("/"))
                .to_path_buf(),
            worker_dir: worker_dir.to_path_buf(),
            local_stores: worker_dir
                .parent()
                .map(|parent| vec![parent.to_path_buf()])
                .unwrap_or_default(),
        }
    }

    struct ScopedEnv {
        _lock: MutexGuard<'static, ()>,
        previous_home: Option<OsString>,
        previous_niles_home: Option<OsString>,
    }

    impl ScopedEnv {
        fn new(niles_home: &Utf8Path, home: &Utf8Path) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            let previous_home = env::var_os("HOME");
            let previous_niles_home = env::var_os("NILES_HOME");
            set_env("NILES_HOME", niles_home.as_str());
            set_env("HOME", home.as_str());
            Self {
                _lock: lock,
                previous_home,
                previous_niles_home,
            }
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            restore_env("HOME", &self.previous_home);
            restore_env("NILES_HOME", &self.previous_niles_home);
        }
    }

    fn set_env(key: &str, value: &str) {
        // SAFETY: These tests hold ENV_LOCK for all NILES_HOME/HOME mutations and resolver calls.
        // They do not spawn additional threads while the scoped environment is active.
        unsafe {
            env::set_var(key, value);
        }
    }

    fn restore_env(key: &str, value: &Option<OsString>) {
        // SAFETY: ScopedEnv still holds ENV_LOCK while restoring the variables it changed.
        unsafe {
            match value {
                Some(value) => env::set_var(key, value),
                None => env::remove_var(key),
            }
        }
    }

    struct TempDir {
        path: Utf8PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = Utf8PathBuf::from_path_buf(env::temp_dir().join(format!(
                "niles-store-{label}-{}-{nanos}",
                std::process::id()
            )))
            .unwrap();
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Utf8Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
