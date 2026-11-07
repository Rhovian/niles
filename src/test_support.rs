//! Test-only scaffolding shared across the crate.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};

mod process;
pub(crate) use process::write_executable as write_executable_script;

pub(crate) fn skill(name: &str) -> crate::skills::SkillName {
    name.parse().unwrap()
}

pub(crate) fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(seconds, 0).unwrap()
}

/// A unique, labeled path under the system temp directory.
pub(crate) fn temp_test_path(label: &str) -> Utf8PathBuf {
    // The clock alone collides across parallel tests: macOS reports microsecond resolution.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
        "niles-test-{label}-{}-{nanos}-{sequence}",
        std::process::id()
    )))
    .unwrap()
}

pub(crate) struct TempDir(Utf8PathBuf);

impl std::ops::Deref for TempDir {
    type Target = Utf8Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<std::path::Path> for TempDir {
    fn as_ref(&self) -> &std::path::Path {
        self.0.as_std_path()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        crate::util::remove_dir_all_if_exists(&self.0).unwrap();
    }
}

pub(crate) fn temp_dir(label: &str) -> TempDir {
    let path = temp_test_path(label);
    std::fs::create_dir_all(&path).unwrap();
    TempDir(path)
}

pub(crate) fn assert_err_contains<T: std::fmt::Debug>(result: anyhow::Result<T>, needle: &str) {
    let error = format!("{:#}", result.unwrap_err());
    assert!(error.contains(needle), "expected {needle:?} in {error}");
}

pub(crate) fn write_worker_meta(
    workspace: &Utf8Path,
    id: &str,
    window: &str,
    created_at: DateTime<Utc>,
    session_link: Option<crate::telemetry::SessionLink>,
) {
    let dir = crate::store::workers_dir(workspace).unwrap().join(id);
    std::fs::create_dir_all(&dir).unwrap();
    crate::store::write_json(
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
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
    terminal.draw(draw).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

pub(crate) fn project_config(body: &str) -> anyhow::Result<crate::config::spec::ProjectConfig> {
    let root = temp_dir("project-config");
    std::fs::write(root.join("niles.yaml"), body)?;
    crate::config::spec::load_project_config_from(&root)
}

/// Remove both the workspace fixture and its global orchestration state.
pub(crate) fn remove_workspace(workspace: &camino::Utf8Path) -> anyhow::Result<()> {
    crate::util::remove_dir_all_if_exists(&crate::store::paths::project_dir(workspace)?)?;
    crate::util::remove_dir_all_if_exists(workspace)
}

/// OnceLock isolates all unit-test home consumers without racing process environment changes.
pub(crate) fn home_dir() -> &'static camino::Utf8Path {
    static HOME_DIR: std::sync::OnceLock<Utf8PathBuf> = std::sync::OnceLock::new();
    HOME_DIR.get_or_init(|| {
        let home = temp_test_path("unit-home");
        let skills = home.join(".niles/skills");
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/skills")).unwrap() {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_dir() {
                continue;
            }
            let source = entry.path();
            let dest = skills.join(source.file_name().unwrap().to_str().unwrap());
            std::fs::create_dir_all(&dest).unwrap();
            std::fs::copy(source.join("SKILL.md"), dest.join("SKILL.md")).unwrap();
        }
        home
    })
}

#[test]
fn orchestration_state_uses_the_private_unit_test_home() {
    let workspace = temp_test_path("isolated-state");
    let home = home_dir();
    assert!(home.is_absolute());
    let state = crate::store::paths::project_dir(&workspace).unwrap();
    assert!(state.starts_with(home.join(".niles/state")));
    assert_eq!(state.file_name(), Some("@state"));
    std::fs::create_dir_all(&state).unwrap();
    assert_eq!(crate::util::home_dir().unwrap(), home);
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

pub(crate) fn manifest() -> crate::workspace_manifest::WorkspaceManifest {
    crate::store::parse_yaml(manifest_body()).unwrap()
}

pub(crate) mod telemetry;
