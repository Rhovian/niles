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
