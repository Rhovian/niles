use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    agents,
    schema::{self, ArtifactKind},
    util::{timestamp_id, write_json_pretty},
    wake,
    workspace_manifest::{self, WorkspaceManifest},
};

use super::startup::startup_context;

const MANAGER_BRIEF_TEMPLATE: &str = include_str!("../templates/manager_brief.md");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default = "default_created_at")]
    pub created_at: chrono::DateTime<Utc>,
    pub workspace: Utf8PathBuf,
    pub brief: Utf8PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<Utf8PathBuf>,
}

pub(super) fn write_manager_session(
    workspace: &Utf8Path,
    agent: &agents::AgentSpec,
    manifest: &WorkspaceManifest,
) -> Result<SessionMeta> {
    let now = Utc::now();
    let id = timestamp_id(&now);
    let dir = workspace.join(".niles").join("sessions").join(&id);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;
    let path = dir.join("manager.md");
    let startup_context = startup_context(workspace)?;
    let body = render_manager_brief(agent, workspace, &dir, manifest, &startup_context);
    fs::write(&path, body).with_context(|| format!("failed to write {path}"))?;
    let meta = SessionMeta {
        id: id.clone(),
        agent: agent.canonical(),
        agent_family: agent.tier().map(|tier| tier.family),
        model: agent.model().map(str::to_owned),
        effort: agent.effort().map(str::to_owned),
        created_at: now,
        workspace: workspace.to_path_buf(),
        brief: path,
        window: None,
        launch: None,
    };
    write_session_meta(workspace, &meta)?;
    fs::write(latest_session_path(workspace), &id)
        .context("failed to write latest session pointer")?;
    Ok(meta)
}

pub(crate) fn read_latest_session(workspace: &Utf8Path) -> Result<Option<SessionMeta>> {
    let Some(id) = crate::util::read_optional_to_string(&latest_session_path(workspace), |path| {
        format!("failed to read latest manager session pointer {path}")
    })?
    else {
        return Ok(None);
    };
    let id = id.trim();
    if id.is_empty() {
        return Ok(None);
    }

    schema::read_optional_json(
        &session_meta_path(workspace, id),
        ArtifactKind::ManagerSession,
    )
}

pub(super) fn write_session_meta(workspace: &Utf8Path, meta: &SessionMeta) -> Result<()> {
    let meta_path = session_meta_path(workspace, &meta.id);
    write_json_pretty(&meta_path, meta)
}

fn render_manager_brief(
    agent: &agents::AgentSpec,
    workspace: &Utf8Path,
    dir: &Utf8Path,
    manifest: &WorkspaceManifest,
    startup_context: &str,
) -> String {
    let manifest_path = workspace_manifest::manifest_path(workspace);
    let flow = workspace_manifest::flow_summary(&manifest.flow);
    MANAGER_BRIEF_TEMPLATE
        .replace("{workspace}", workspace.as_str())
        .replace("{agent}", &agent.canonical())
        .replace("{dir}", dir.as_str())
        .replace("{manifest}", manifest_path.as_str())
        .replace("{flow}", &flow)
        .replace("{startup_context}", startup_context)
        .replace(
            "{worker_wake_examples}",
            &wake::manager_worker_contract_examples("<status-file>"),
        )
}

fn session_meta_path(workspace: &Utf8Path, id: &str) -> Utf8PathBuf {
    workspace
        .join(".niles")
        .join("sessions")
        .join(id)
        .join("session.json")
}

fn latest_session_path(workspace: &Utf8Path) -> Utf8PathBuf {
    workspace.join(".niles").join("sessions").join("latest")
}

fn default_created_at() -> chrono::DateTime<Utc> {
    Utc::now()
}

#[cfg(test)]
mod tests {
    use super::super::test_support::temp_test_path;
    use super::*;

    #[test]
    fn manager_brief_omits_removed_manifest_command() {
        assert!(MANAGER_BRIEF_TEMPLATE.contains("niles spawn <id>"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("niles run"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("manifest: {manifest}"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("flow: {flow}"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("source of truth"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("standard worker-verification-reviewer loop"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("CONSENSUS OR ESCALATE"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("Do not generate a task YAML file"));
        assert!(!MANAGER_BRIEF_TEMPLATE.contains("niles manifest"));
    }

    #[test]
    fn cost_discipline_guidance() {
        assert_eq!(
            MANAGER_BRIEF_TEMPLATE.matches("## Cost Discipline").count(),
            1
        );
        assert!(MANAGER_BRIEF_TEMPLATE.contains("Tier reviewers by surface risk"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("Scope re-reviews to the fix delta"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("Keep manager context lean"));
    }

    #[test]
    fn manager_brief_render_includes_manifest_flow() {
        let workspace = temp_test_path("brief-render");
        let dir = workspace.join(".niles/sessions/test-session");
        let agent = agents::parse_spec("codex:gpt-5.5:xhigh").unwrap();
        let manifest = WorkspaceManifest {
            manager: "codex:gpt-5.5:xhigh".to_owned(),
            planner: "planbot".to_owned(),
            worker: "codebot".to_owned(),
            reviewer: "reviewbot".to_owned(),
            validation_command: "check".to_owned(),
            flow: vec![
                workspace_manifest::WorkspaceFlowRole::Reviewer,
                workspace_manifest::WorkspaceFlowRole::Validation,
            ],
        };

        let body = render_manager_brief(&agent, &workspace, &dir, &manifest, "latest_run: none");

        assert!(body.contains(&format!(
            "manifest: {}",
            workspace_manifest::manifest_path(&workspace)
        )));
        assert!(body.contains("flow: reviewer -> validation"));
        assert!(body.contains("manager_agent: codex:gpt-5.5:xhigh"));
        assert!(body.contains("latest_run: none"));
        assert!(!body.contains("{manifest}"));
        assert!(!body.contains("{flow}"));
    }
}
