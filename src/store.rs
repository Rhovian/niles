use std::{collections::BTreeMap, env, fs, path::PathBuf};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::{
    state::{RunState, StepRecord},
    util::{absolute_path, current_dir_utf8, read_optional_json, utf8_path, write_json_pretty},
};

const NILES_DIR: &str = ".niles";
const RUNS_DIR: &str = "runs";
const LATEST_POINTER: &str = "latest.json";

pub fn write_state(path: &Utf8Path, state: &RunState) -> Result<()> {
    write_json_pretty(path, state)
}

pub fn read_state(run_dir: &Utf8Path) -> Result<RunState> {
    let path = state_path(run_dir);
    let body = fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
    serde_json::from_str(&body).with_context(|| format!("failed to parse {path}"))
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

pub fn register_run_location(run: &str, workspace: &Utf8Path, run_dir: &Utf8Path) -> Result<()> {
    let pointer = RunPointer {
        id: run.to_owned(),
        workspace: workspace.to_path_buf(),
        run_dir: run_dir.to_path_buf(),
    };

    write_local_pointer(&current_runs_dir()?, &pointer)?;
    write_local_pointer(&workspace.join(NILES_DIR).join(RUNS_DIR), &pointer)?;
    write_global_pointer(&pointer)
}

pub fn resolve_run_dir(run: &str) -> Result<Utf8PathBuf> {
    let resolver = RunResolver::from_current()?;
    if run == "latest" {
        return resolver.latest()?.context("no runs found");
    }

    resolver.named(run)
}

pub fn resolve_latest_run_dir() -> Result<Option<Utf8PathBuf>> {
    RunResolver::from_current()?.latest()
}

fn current_runs_dir() -> Result<Utf8PathBuf> {
    Ok(current_dir_utf8()?.join(NILES_DIR).join(RUNS_DIR))
}

fn write_local_pointer(runs_dir: &Utf8Path, pointer: &RunPointer) -> Result<()> {
    fs::create_dir_all(runs_dir).with_context(|| format!("failed to create {runs_dir}"))?;
    write_pointer(runs_dir, PointerFile::Run(&pointer.id), pointer)?;
    write_pointer(runs_dir, PointerFile::Latest, pointer)
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
        if let Some(run_dir) = self.local_pointer(PointerFile::Run(run))? {
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

    fn latest(&self) -> Result<Option<Utf8PathBuf>> {
        if let Some(run_dir) = self.local_pointer(PointerFile::Latest)? {
            return Ok(Some(run_dir));
        }
        if let Some(run_dir) = latest_local_run_dir(&self.local_runs_dir)? {
            return Ok(Some(run_dir));
        }
        latest_global_run_dir()
    }

    fn local_pointer(&self, pointer_file: PointerFile<'_>) -> Result<Option<Utf8PathBuf>> {
        read_pointer(&pointer_file.path(&self.local_runs_dir))
            .map(|pointer| pointer.map(|pointer| pointer.run_dir))
    }
}

#[derive(Clone, Copy)]
enum PointerFile<'a> {
    Run(&'a str),
    Latest,
}

impl PointerFile<'_> {
    fn path(self, runs_dir: &Utf8Path) -> Utf8PathBuf {
        match self {
            PointerFile::Run(run) => runs_dir.join(pointer_file(run)),
            PointerFile::Latest => runs_dir.join(LATEST_POINTER),
        }
    }
}

fn write_pointer(
    runs_dir: &Utf8Path,
    pointer_file: PointerFile<'_>,
    pointer: &RunPointer,
) -> Result<()> {
    write_json_pretty(&pointer_file.path(runs_dir), pointer)
}

fn latest_local_run_dir(runs_dir: &Utf8Path) -> Result<Option<Utf8PathBuf>> {
    let entries = match fs::read_dir(runs_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to read {runs_dir}")),
    };

    let mut runs = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = Utf8PathBuf::from_path_buf(entry.path()).ok()?;
            path.is_dir().then_some(path)
        })
        .collect::<Vec<_>>();

    runs.sort();
    Ok(runs.pop())
}

fn read_pointer(path: &Utf8Path) -> Result<Option<RunPointer>> {
    read_optional_json(
        path,
        |path| format!("failed to read run pointer {path}"),
        |path| format!("failed to parse run pointer {path}"),
    )
}

fn write_global_pointer(pointer: &RunPointer) -> Result<()> {
    let path = global_index_path()?;
    let mut index = read_global_index(&path)?;
    index.runs.insert(pointer.id.clone(), pointer.clone());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    }
    write_json_pretty(&path, &index)
}

fn resolve_global_run(run: &str) -> Result<Option<Utf8PathBuf>> {
    let path = global_index_path()?;
    Ok(read_global_index(&path)?
        .runs
        .get(run)
        .map(|pointer| pointer.run_dir.clone()))
}

fn latest_global_run_dir() -> Result<Option<Utf8PathBuf>> {
    let path = global_index_path()?;
    Ok(read_global_index(&path)?
        .runs
        .into_iter()
        .next_back()
        .map(|(_, pointer)| pointer.run_dir))
}

fn read_global_index(path: &Utf8Path) -> Result<RunIndex> {
    Ok(read_optional_json(
        path,
        |path| format!("failed to read {path}"),
        |path| format!("failed to parse {path}"),
    )?
    .unwrap_or_default())
}

fn global_index_path() -> Result<Utf8PathBuf> {
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

#[derive(Default, Deserialize, Serialize)]
struct RunIndex {
    #[serde(default)]
    runs: BTreeMap<String, RunPointer>,
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

        assert!(named_error.contains("failed to parse run pointer"));
        assert!(latest_error.contains("failed to parse run pointer"));
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
    fn stale_local_pointers_return_recorded_path_without_existence_check() {
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
        assert_eq!(resolver.latest().unwrap(), Some(stale_target));
    }

    fn resolver_at(local_runs_dir: &Utf8Path) -> RunResolver {
        RunResolver {
            local_runs_dir: local_runs_dir.to_path_buf(),
        }
    }

    fn create_dir(path: Utf8PathBuf) -> Utf8PathBuf {
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_local_run_pointer(runs_dir: &Utf8Path, run: &str, run_dir: &Utf8Path) {
        write_pointer(runs_dir, PointerFile::Run(run), &run_pointer(run, run_dir)).unwrap();
    }

    fn write_latest_pointer(runs_dir: &Utf8Path, run: &str, run_dir: &Utf8Path) {
        write_pointer(runs_dir, PointerFile::Latest, &run_pointer(run, run_dir)).unwrap();
    }

    fn write_global_run_pointer(run: &str, run_dir: &Utf8Path) {
        write_global_pointer(&run_pointer(run, run_dir)).unwrap();
    }

    fn write_index(path: &Utf8Path, pointers: &[RunPointer]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }

        let mut index = RunIndex::default();
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

    struct ScopedEnv {
        _lock: MutexGuard<'static, ()>,
        previous_home: Option<OsString>,
        previous_niles_home: Option<OsString>,
    }

    impl ScopedEnv {
        fn new(niles_home: &Utf8Path, home: &Utf8Path) -> Self {
            let lock = ENV_LOCK.lock().unwrap();
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
