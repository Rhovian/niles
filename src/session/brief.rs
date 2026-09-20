use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    agents,
    store::paths::NILES_DIR,
    util::{timestamp_id, write_json_pretty},
    workspace_manifest,
};

use super::startup::startup_context;

const LEAD_BRIEF_TEMPLATE: &str = include_str!("../templates/lead_brief.md");

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
}

pub(super) fn write_manager_session(
    workspace: &Utf8Path,
    agent: &agents::AgentSpec,
) -> Result<SessionMeta> {
    let now = Utc::now();
    let id = timestamp_id(&now);
    let dir = workspace.join(NILES_DIR).join("sessions").join(&id);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;
    let path = dir.join("lead.md");
    let startup_context = startup_context(workspace)?;
    let body = render_lead_brief(agent, workspace, &dir, &startup_context);
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
    };
    write_session_meta(workspace, &meta)?;
    fs::write(latest_session_path(workspace), &id)
        .context("failed to write latest session pointer")?;
    Ok(meta)
}

pub(super) fn write_session_meta(workspace: &Utf8Path, meta: &SessionMeta) -> Result<()> {
    let meta_path = session_meta_path(workspace, &meta.id);
    write_json_pretty(&meta_path, meta)
}

fn render_lead_brief(
    agent: &agents::AgentSpec,
    workspace: &Utf8Path,
    dir: &Utf8Path,
    startup_context: &str,
) -> String {
    let manifest_path = workspace_manifest::manifest_path(workspace);
    LEAD_BRIEF_TEMPLATE
        .replace("{workspace}", workspace.as_str())
        .replace("{agent}", &agent.canonical())
        .replace("{dir}", dir.as_str())
        .replace("{manifest}", manifest_path.as_str())
        .replace("{startup_context}", startup_context)
}

fn session_meta_path(workspace: &Utf8Path, id: &str) -> Utf8PathBuf {
    workspace
        .join(NILES_DIR)
        .join("sessions")
        .join(id)
        .join("session.json")
}

fn latest_session_path(workspace: &Utf8Path) -> Utf8PathBuf {
    workspace.join(NILES_DIR).join("sessions").join("latest")
}

fn default_created_at() -> chrono::DateTime<Utc> {
    Utc::now()
}

#[cfg(test)]
mod tests {
    use super::super::test_support::temp_test_path;
    use super::*;

    /// Planning is the lead's, implementation is not. With no planner role left, a plan
    /// derived twice is the duplication #89 describes.
    #[test]
    fn lead_brief_keeps_the_plan_and_delegates_the_implementation() {
        assert!(LEAD_BRIEF_TEMPLATE.contains("You own the outcome and the plan"));
        assert!(LEAD_BRIEF_TEMPLATE.contains("hand a worker a plan rather than a puzzle"));
        assert!(LEAD_BRIEF_TEMPLATE.contains("Do not implement."));
        // The undershoot half: dispatching a worker for a check it could finish itself (#121).
        assert!(
            LEAD_BRIEF_TEMPLATE.contains("Do inline whatever is cheaper to do than to delegate")
        );
    }

    /// The audit is commissioned deliberately, not folded into every review (#119).
    #[test]
    fn lead_brief_commissions_security_only_for_security_boundaries() {
        assert!(LEAD_BRIEF_TEMPLATE.contains("will not write hardening findings"));
        assert!(LEAD_BRIEF_TEMPLATE.contains("only when the change is itself a security boundary"));
    }

    /// The gate has exactly one owner, and it is not the lead (the token-burn fix).
    #[test]
    fn lead_brief_assigns_the_gate_to_the_worker() {
        assert!(LEAD_BRIEF_TEMPLATE.contains("The gate belongs to the worker"));
        assert!(LEAD_BRIEF_TEMPLATE.contains("the same command a third time, not verification"));
    }

    /// Naming model tiers in the brief dates it to a release (#90).
    #[test]
    fn lead_brief_tiers_effort_without_naming_models() {
        for model in ["opus", "sonnet", "gpt-5", "claude:", "codex:gpt"] {
            assert!(
                !LEAD_BRIEF_TEMPLATE.contains(model),
                "lead brief should not hardcode the model ladder, found {model:?}"
            );
        }
        assert!(LEAD_BRIEF_TEMPLATE.contains("Effort follows risk"));
        assert!(LEAD_BRIEF_TEMPLATE.contains("Scope a re-review to the fix"));
    }

    /// The brief is read on every session start; length is a running cost.
    #[test]
    fn lead_brief_stays_short() {
        let lines = LEAD_BRIEF_TEMPLATE.lines().count();
        assert!(lines <= 50, "lead brief is {lines} lines; keep it tight");
    }

    #[test]
    fn lead_brief_keeps_delegation_inside_niles() {
        assert!(LEAD_BRIEF_TEMPLATE.contains("Delegation goes through niles"));
        assert!(LEAD_BRIEF_TEMPLATE.contains("niles spawn <id> --role"));
        // The command surface is fetched, not carried: it costs tokens at every session start.
        assert!(LEAD_BRIEF_TEMPLATE.contains("niles <command> --help"));
        for fetchable in [
            "niles peek <id>",
            "niles workers",
            "niles close <id>",
            "niles wait <id>",
        ] {
            assert!(
                !LEAD_BRIEF_TEMPLATE.contains(fetchable),
                "{fetchable} is in spawn output and --help; do not carry it in the brief"
            );
        }
    }

    #[test]
    fn lead_brief_render_fills_every_placeholder() {
        let workspace = temp_test_path("brief-render");
        let dir = workspace.join(".niles/sessions/test-session");
        let agent = agents::parse_spec("codex:gpt-5.5:xhigh").unwrap();

        let body = render_lead_brief(&agent, &workspace, &dir, "worker: none");

        assert!(body.contains(&format!(
            "manifest: {}",
            workspace_manifest::manifest_path(&workspace)
        )));
        assert!(body.contains("lead_agent: codex:gpt-5.5:xhigh"));
        assert!(body.contains("worker: none"));
        assert!(!body.contains("{manifest}"), "unfilled placeholder: {body}");
        assert!(
            !body.contains("{workspace}"),
            "unfilled placeholder: {body}"
        );
        assert!(!body.contains("{agent}"), "unfilled placeholder: {body}");
        assert!(!body.contains("{dir}"), "unfilled placeholder: {body}");
        assert!(
            !body.contains("{startup_context}"),
            "unfilled placeholder: {body}"
        );
    }
}
