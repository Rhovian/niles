use anyhow::{Context, Result, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{store, telemetry::SessionLink};

use crate::skills::SkillName;

const REPORT_FILE: &str = "report.md";

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct WorkerMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) setup: Option<crate::workspace_manifest::setup::Setup>,
    pub(super) id: String,
    pub(super) skill: SkillName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) gates: Option<crate::workspace_manifest::Gates>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) menu: Vec<SkillName>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub(super) imports: std::collections::BTreeMap<SkillName, crate::skills::reference::PinnedRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) always: Vec<SkillName>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) on_from: Vec<SkillName>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) on_stage: Vec<SkillName>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) with: Vec<SkillName>,
    pub(super) agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) agent_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) task_label: Option<String>,
    pub(super) created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) starting_until: Option<DateTime<Utc>>,
    pub(super) project: Utf8PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) tree: Option<Utf8PathBuf>,
    pub(super) window: String,
    pub(super) brief: Utf8PathBuf,
    pub(super) launch: Utf8PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) session_link: Option<SessionLink>,
}

impl WorkerMeta {
    pub(super) fn agent_dir(&self) -> &Utf8Path {
        match &self.tree {
            Some(tree) => tree,
            None => &self.project,
        }
    }
}

pub(super) fn write_meta(worker_dir: &Utf8Path, meta: &WorkerMeta) -> Result<()> {
    let path = meta_path(worker_dir);
    store::write_json(&path, meta)
}

pub(super) fn read_meta(worker_dir: &Utf8Path) -> Result<WorkerMeta> {
    let path = meta_path(worker_dir);
    let meta = read_meta_if_exists(worker_dir)?
        .with_context(|| format!("worker metadata missing at {path}"))?;
    Ok(meta)
}

pub(super) fn read_meta_if_exists(worker_dir: &Utf8Path) -> Result<Option<WorkerMeta>> {
    let path = meta_path(worker_dir);
    let meta: Option<WorkerMeta> = store::read_optional_json(&path)?;
    if let Some(meta) = &meta {
        for (name, reference) in &meta.imports {
            ensure!(
                name == &reference.reference().name && meta.menu.contains(name),
                "invalid ephemeral import {name} in {path}"
            );
        }
    }
    Ok(meta)
}

pub(super) fn meta_path(worker_dir: &Utf8Path) -> Utf8PathBuf {
    worker_dir.join("meta.json")
}

pub(super) fn report_path(worker_dir: &Utf8Path) -> Utf8PathBuf {
    worker_dir.join(REPORT_FILE)
}

pub(super) fn worker_dir_from_env() -> Result<Utf8PathBuf> {
    let dir = Utf8PathBuf::from(
        std::env::var("NILES_AGENT_DIR").context("NILES_AGENT_DIR is missing or invalid")?,
    );
    ensure!(
        dir.is_absolute() && dir.is_dir(),
        "NILES_AGENT_DIR must be an absolute worker directory: {dir}"
    );
    Ok(dir)
}
