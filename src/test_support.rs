//! Test-only scaffolding shared across the crate.
use crate::config::spec::{self, ProjectConfig};
use crate::skills::SkillName;
use crate::store::{self, atomic_write, paths};
use crate::telemetry::SessionLink;
use crate::util;
use crate::workspace_manifest::WorkspaceManifest;
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use ratatui::backend::TestBackend;
use std::fmt::Debug;
use std::ops::Deref;
use std::path::Path;
use std::sync::OnceLock;
use std::{env, fs};

mod process;
mod skills;
pub(crate) use process::write_executable as write_executable_script;

pub(crate) fn skill(name: &str) -> SkillName {
    name.parse().unwrap()
}

pub(crate) fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(seconds, 0).unwrap()
}

/// A unique, labeled path under the system temp directory.
pub(crate) fn temp_test_path(label: &str) -> Utf8PathBuf {
    Utf8PathBuf::from_path_buf(
        env::temp_dir().join(format!("niles-test-{label}-{}", uuid::Uuid::new_v4())),
    )
    .unwrap()
}

pub(crate) struct TempDir(Utf8PathBuf);

impl Deref for TempDir {
    type Target = Utf8Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        self.0.as_std_path()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        util::remove_dir_all_if_exists(&self.0).unwrap();
    }
}

pub(crate) fn temp_dir(label: &str) -> TempDir {
    let path = temp_test_path(label);
    fs::create_dir_all(&path).unwrap();
    TempDir(path)
}

pub(crate) fn assert_err_contains<T: Debug>(result: anyhow::Result<T>, needle: &str) {
    let error = format!("{:#}", result.unwrap_err());
    assert!(error.contains(needle), "expected {needle:?} in {error}");
}

pub(crate) fn write_worker_meta(
    workspace: &Utf8Path,
    id: &str,
    window: &str,
    created_at: DateTime<Utc>,
    session_link: Option<SessionLink>,
) {
    let dir = store::workers_dir(workspace).unwrap().join(id);
    fs::create_dir_all(&dir).unwrap();
    store::write_json(
        &dir.join("meta.json"),
        &serde_json::json!({
            "id": id, "skill": "worker", "agent": "codex", "created_at": created_at,
            "project": workspace, "window": window, "brief": dir.join("brief.md"),
            "launch": dir.join("launch.sh"), "session_link": session_link,
        }),
    )
    .unwrap();
}

pub(crate) fn render(w: u16, h: u16, draw: impl FnOnce(&mut ratatui::Frame<'_>)) -> String {
    let mut terminal = ratatui::Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(draw).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

pub(crate) fn project_config(body: &str) -> anyhow::Result<ProjectConfig> {
    let root = temp_dir("project-config");
    atomic_write(root.join("niles.yaml"), body)?;
    spec::load_project_config_from(&root)
}

/// Remove both the workspace fixture and its global orchestration state.
pub(crate) fn remove_workspace(workspace: &camino::Utf8Path) -> anyhow::Result<()> {
    util::remove_dir_all_if_exists(&paths::project_dir(workspace)?)?;
    util::remove_dir_all_if_exists(workspace)
}

/// OnceLock isolates all unit-test home consumers without racing process environment changes.
pub(crate) fn home_dir() -> &'static camino::Utf8Path {
    static HOME_DIR: OnceLock<Utf8PathBuf> = OnceLock::new();
    HOME_DIR.get_or_init(|| {
        let home = temp_test_path("unit-home");
        skills::copy_skills(home.join(".niles").as_std_path());
        home
    })
}

#[test]
fn orchestration_state_uses_the_private_unit_test_home() {
    let workspace = temp_test_path("isolated-state");
    let home = home_dir();
    assert!(home.is_absolute());
    let state = paths::project_dir(&workspace).unwrap();
    assert!(state.starts_with(home.join(".niles/state")));
    assert_eq!(state.file_name(), Some("@state"));
    fs::create_dir_all(&state).unwrap();
    assert_eq!(util::home_dir().unwrap(), home);
    remove_workspace(&workspace).unwrap();
    assert!(!state.exists());
}

pub(crate) mod default_manifest {
    use crate::workspace_manifest::{self, WorkspaceManifest};
    use anyhow::Result;
    use camino::{Utf8Path, Utf8PathBuf};

    pub(crate) fn load(root: &Utf8Path) -> Result<Option<WorkspaceManifest>> {
        workspace_manifest::load(root, None)
    }
    pub(crate) fn save(root: &Utf8Path, manifest: &WorkspaceManifest) -> Result<()> {
        workspace_manifest::save(root, manifest, None)
    }
    pub(crate) fn manifest_path(root: &Utf8Path) -> Result<Utf8PathBuf> {
        workspace_manifest::manifest_path(root, None)
    }
}

pub(crate) fn manifest_body() -> &'static str {
    include_str!("test_support/manifest.yaml")
}

pub(crate) fn manifest() -> WorkspaceManifest {
    store::parse_yaml(manifest_body()).unwrap()
}

pub(crate) mod telemetry;

mod assertions;
pub(crate) use assertions::*;

#[cfg(feature = "remote")]
mod json;
#[cfg(feature = "remote")]
pub(crate) use json::json_field;
