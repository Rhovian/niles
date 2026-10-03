use std::{env, fs};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    agents,
    telemetry::SessionLink,
    util::{read_dir_utf8_paths, render_template, timestamp_id},
    workspace_manifest::{self, ReviewerBinding, WorkspaceManifest},
};

use super::{sessions_dir, startup::startup_context};

const LEAD_BRIEF_TEMPLATE: &str = include_str!("../templates/lead_brief.md");
const LEAD_REVIEW_TEMPLATE: &str = include_str!("../templates/lead_review.md");
const COMMISSION_REVIEW_TEMPLATE: &str = include_str!("../templates/commission_review.md");
const REVIEWER_STANDARD: &str = include_str!("../templates/role_reviewer.md");

/// The environment variable tmux sets for the pane a process runs in.
const LEAD_PANE_ENV: &str = "TMUX_PANE";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SessionMeta {
    pub id: String,
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
    pub workspace: Utf8PathBuf,
    pub brief: Utf8PathBuf,
    /// The pane the lead is running in, recorded once from `$TMUX_PANE` at session start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead_pane: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_link: Option<SessionLink>,
}

pub(super) struct ManagerSession {
    pub meta: SessionMeta,
    pub brief: String,
    pub dir: Utf8PathBuf,
}

pub(super) fn write_manager_session(
    workspace: &Utf8Path,
    agent: &agents::AgentSpec,
    manifest: &WorkspaceManifest,
) -> Result<ManagerSession> {
    let now = Utc::now();
    let id = timestamp_id(&now);
    let dir = sessions_dir(workspace).join(&id);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;
    let path = dir.join("lead.md");
    let startup_context = startup_context(workspace)?;
    let body = render_lead_brief(agent, workspace, &dir, &startup_context, &manifest.reviewer);
    fs::write(&path, &body).with_context(|| format!("failed to write {path}"))?;
    let meta = SessionMeta {
        id: id.clone(),
        agent: agent.canonical(),
        agent_family: agent.tiered_family(),
        model: agent.model().map(str::to_owned),
        effort: agent.effort().map(str::to_owned),
        created_at: now,
        workspace: workspace.to_path_buf(),
        brief: path,
        lead_pane: recorded_lead_pane(),
        session_link: agents::session_link(agent.family(), &format!("lead-{id}"), &dir),
    };
    write_session_meta(workspace, &meta)?;
    Ok(ManagerSession {
        meta,
        brief: body,
        dir,
    })
}

pub(super) fn write_session_meta(workspace: &Utf8Path, meta: &SessionMeta) -> Result<()> {
    let meta_path = session_meta_path(workspace, &meta.id);
    crate::store::write_json(&meta_path, meta)
}

fn render_lead_brief(
    agent: &agents::AgentSpec,
    workspace: &Utf8Path,
    dir: &Utf8Path,
    startup_context: &str,
    reviewer: &ReviewerBinding,
) -> String {
    let manifest_path = workspace_manifest::manifest_path(workspace);
    let (review_instruction, reviewer_standard) = match reviewer {
        ReviewerBinding::Lead => (LEAD_REVIEW_TEMPLATE.trim_end(), Some(REVIEWER_STANDARD)),
        ReviewerBinding::Agent(_) => (COMMISSION_REVIEW_TEMPLATE.trim_end(), None),
    };
    let mut body = render_template(
        LEAD_BRIEF_TEMPLATE,
        &[
            ("{workspace}", workspace.as_str()),
            ("{agent}", &agent.canonical()),
            ("{dir}", dir.as_str()),
            ("{manifest}", manifest_path.as_str()),
            ("{startup_context}", startup_context),
            ("{review_instruction}", review_instruction),
        ],
    );
    if let Some(standard) = reviewer_standard {
        body.push('\n');
        body.push_str(standard);
    }
    body
}

fn session_meta_path(workspace: &Utf8Path, id: &str) -> Utf8PathBuf {
    sessions_dir(workspace).join(id).join("session.json")
}

pub(crate) fn live_lead(workspace: &Utf8Path) -> Result<Option<SessionMeta>> {
    let Some(pane) = recorded_lead_pane() else {
        return Ok(None);
    };
    Ok(latest_lead(workspace)?.filter(|meta| meta.lead_pane.as_deref() == Some(&pane)))
}

pub(crate) fn latest_lead(workspace: &Utf8Path) -> Result<Option<SessionMeta>> {
    for dir in read_dir_utf8_paths(&sessions_dir(workspace))?
        .into_iter()
        .rev()
    {
        if dir.is_dir()
            && let Some(meta) = crate::store::read_optional_json(&dir.join("session.json"))?
        {
            return Ok(Some(meta));
        }
    }
    Ok(None)
}

/// `$TMUX_PANE` as tmux set it for the pane this process is running in: a `%N` pane id, which is a
/// complete tmux target on its own. Absent or empty outside tmux, which is how a session with no
/// pane of its own is written.
fn recorded_lead_pane() -> Option<String> {
    match env::var(LEAD_PANE_ENV) {
        Ok(pane) if !pane.trim().is_empty() => Some(pane),
        Ok(_) | Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_path;

    /// The brief is read on every session start; length is a running cost.
    #[test]
    fn lead_brief_stays_short() {
        let lines = LEAD_BRIEF_TEMPLATE.lines().count();
        assert!(lines <= 60, "lead brief is {lines} lines; keep it tight");
    }

    #[test]
    fn lead_brief_does_not_name_model_tiers() {
        for model in ["opus", "sonnet", "gpt-5", "claude:", "codex:gpt"] {
            assert!(
                !LEAD_BRIEF_TEMPLATE.contains(model),
                "lead brief should not hardcode the model ladder, found {model:?}"
            );
        }
    }

    #[test]
    fn lead_brief_does_not_duplicate_fetched_commands() {
        for command in ["niles peek <id>", "niles close <id>", "niles wait <id>"] {
            assert!(
                !LEAD_BRIEF_TEMPLATE.contains(command),
                "{command} is available from spawn output and --help"
            );
        }
    }

    #[test]
    fn session_meta_records_the_lead_pane() {
        let workspace = temp_test_path("session-lead-pane");
        fs::create_dir_all(&workspace).unwrap();
        let mut meta = SessionMeta {
            id: "session".to_owned(),
            agent: "claude".to_owned(),
            agent_family: None,
            model: None,
            effort: None,
            created_at: Utc::now(),
            workspace: workspace.clone(),
            brief: workspace.join("lead.md"),
            lead_pane: Some("%7".to_owned()),
            session_link: None,
        };
        let file = session_meta_path(&workspace, &meta.id);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        write_session_meta(&workspace, &meta).unwrap();

        let body = fs::read_to_string(&file).unwrap();
        assert!(body.contains("\"lead_pane\": \"%7\""), "{body}");

        // A session with no pane omits the field rather than writing a null nobody records.
        meta.lead_pane = None;
        write_session_meta(&workspace, &meta).unwrap();
        let body = fs::read_to_string(&file).unwrap();
        assert!(!body.contains("lead_pane"), "{body}");

        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn lead_brief_render_fills_every_placeholder() {
        let workspace = temp_test_path("brief-{dir}-render");
        let dir = workspace.join(".niles/sessions/test-session");
        let models = agents::ModelRoster::builtin().unwrap();
        let agent = agents::AgentSpec::parse("codex:gpt-5.5:xhigh", &models).unwrap();

        let body = render_lead_brief(
            &agent,
            &workspace,
            &dir,
            "worker: none",
            &ReviewerBinding::Agent("claude".to_owned()),
        );

        assert!(body.contains(&format!(
            "manifest: {}",
            workspace_manifest::manifest_path(&workspace)
        )));
        assert!(body.contains("lead_agent: codex:gpt-5.5:xhigh"));
        assert!(body.contains("worker: none"));
        assert!(body.contains(&format!("workspace: {workspace}\n")));
        assert!(body.contains(&format!("session_dir: {dir}\n")));
        for placeholder in ["{manifest}", "{workspace}", "{agent}", "{startup_context}"] {
            assert!(!body.contains(placeholder), "unfilled placeholder: {body}");
        }
        let lead = render_lead_brief(
            &agent,
            &workspace,
            &dir,
            "worker: none",
            &ReviewerBinding::Lead,
        );
        assert!(lead.contains(REVIEWER_STANDARD));
    }
}
