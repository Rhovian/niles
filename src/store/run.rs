use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::{
    schema::{self, ArtifactKind},
    util::{read_dir_utf8_paths, write_json_pretty},
};

use super::{
    global::{latest_global_run_dir, resolve_global_run, write_global_run_pointer},
    paths::{
        LATEST_POINTER, NILES_DIR, RUNS_DIR, current_runs_dir, pointer_file, workspace_runs_dir,
    },
};

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

fn write_local_run_pointer(runs_dir: &Utf8Path, pointer: &RunPointer) -> Result<()> {
    fs::create_dir_all(runs_dir).with_context(|| format!("failed to create {runs_dir}"))?;
    write_run_pointer(runs_dir, RunPointerFile::Run(&pointer.id), pointer)?;
    write_run_pointer(runs_dir, RunPointerFile::Latest, pointer)
}

pub(super) struct RunResolver {
    pub(super) local_runs_dir: Utf8PathBuf,
}

impl RunResolver {
    fn from_current() -> Result<Self> {
        Ok(Self {
            local_runs_dir: current_runs_dir()?,
        })
    }

    pub(super) fn named(&self, run: &str) -> Result<Utf8PathBuf> {
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

#[derive(Clone, Copy)]
pub(super) enum RunPointerFile<'a> {
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

pub(super) fn write_run_pointer(
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

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct RunPointer {
    pub(super) id: String,
    pub(super) workspace: Utf8PathBuf,
    pub(super) run_dir: Utf8PathBuf,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::store::{
        paths::{LATEST_POINTER, pointer_file},
        test_support::{
            ScopedEnv, TempDir, create_dir, resolver_at, write_global_run_pointer,
            write_latest_pointer, write_local_run_pointer,
        },
    };

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

        fs::remove_file(crate::store::global_index_path().unwrap()).unwrap();
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

        fs::remove_file(crate::store::global_index_path().unwrap()).unwrap();
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
}
