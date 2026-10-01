use std::{env, fs};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use serde::Serialize;

use crate::{
    agents,
    util::{render_template, timestamp_id},
    workspace_manifest,
};

use super::{sessions_dir, startup::startup_context};

const LEAD_BRIEF_TEMPLATE: &str = include_str!("../templates/lead_brief.md");

/// The environment variable tmux sets for the pane a process runs in.
const LEAD_PANE_ENV: &str = "TMUX_PANE";

#[derive(Debug, Clone, Serialize)]
pub(super) struct SessionMeta {
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
}

pub(super) struct ManagerSession {
    pub meta: SessionMeta,
    pub brief: String,
    pub dir: Utf8PathBuf,
}

pub(super) fn write_manager_session(
    workspace: &Utf8Path,
    agent: &agents::AgentSpec,
) -> Result<ManagerSession> {
    let now = Utc::now();
    let id = timestamp_id(&now);
    let dir = sessions_dir(workspace).join(&id);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;
    let path = dir.join("lead.md");
    let startup_context = startup_context(workspace)?;
    let body = render_lead_brief(agent, workspace, &dir, &startup_context);
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
) -> String {
    let manifest_path = workspace_manifest::manifest_path(workspace);
    render_template(
        LEAD_BRIEF_TEMPLATE,
        &[
            ("{workspace}", workspace.as_str()),
            ("{agent}", &agent.canonical()),
            ("{dir}", dir.as_str()),
            ("{manifest}", manifest_path.as_str()),
            ("{startup_context}", startup_context),
        ],
    )
}

fn session_meta_path(workspace: &Utf8Path, id: &str) -> Utf8PathBuf {
    sessions_dir(workspace).join(id).join("session.json")
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

        let body = render_lead_brief(&agent, &workspace, &dir, "worker: none");

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
    }
}
