//! Test-only scaffolding shared across the crate.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};

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

pub(crate) fn project_config(body: &str) -> anyhow::Result<crate::config::spec::ProjectConfig> {
    let root = temp_test_path("project-config");
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join("niles.yaml"), body)?;
    let result = crate::config::spec::load_project_config_from(&root);
    std::fs::remove_dir_all(root)?;
    result
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
    assert_eq!(
        state,
        home.join(".niles/state")
            .join(workspace.as_str().replace('/', "-"))
    );
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
    include_str!("../skills/niles-setup/SKILL.md")
        .split_once("```yaml\n")
        .unwrap()
        .1
        .split_once("```")
        .unwrap()
        .0
}

pub(crate) fn manifest() -> crate::workspace_manifest::WorkspaceManifest {
    crate::store::parse_yaml(manifest_body()).unwrap()
}
