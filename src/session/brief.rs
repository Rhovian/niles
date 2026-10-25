use std::{env, fs};

use anyhow::{Context, Result, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    agents,
    skills::SkillName,
    telemetry::SessionLink,
    util::{read_dir_utf8_paths, render_template, timestamp_id},
    workspace_manifest,
};

use super::{sessions_dir, startup::startup_context};

/// The environment variable tmux sets for the pane a process runs in.
const LEAD_PANE_ENV: &str = "TMUX_PANE";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SessionMeta {
    pub id: String,
    pub agent: String,
    pub entry_skill: SkillName,
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
    pub dir: Utf8PathBuf,
    pub brief: String,
}

pub(super) fn prepare_manager_session(
    workspace: &Utf8Path,
    agent: &agents::AgentSpec,
    skill: &SkillName,
    template: &str,
) -> Result<ManagerSession> {
    let now = Utc::now();
    let id = timestamp_id(&now);
    let dir = sessions_dir(workspace)?.join(&id);
    let path = dir.join("lead.md");
    let startup_context = startup_context(workspace)?;
    let body = render_entry_brief(agent, workspace, &dir, &startup_context, template)?;
    let meta = SessionMeta {
        id: id.clone(),
        agent: agent.canonical(),
        entry_skill: skill.clone(),
        agent_family: agent.tiered_family(),
        model: agent.model().map(str::to_owned),
        effort: agent.effort().map(str::to_owned),
        created_at: now,
        workspace: workspace.to_path_buf(),
        brief: path,
        lead_pane: recorded_lead_pane(),
        session_link: agents::session_link(agent.family(), &format!("lead-{id}"), &dir),
    };
    Ok(ManagerSession {
        meta,
        dir,
        brief: body,
    })
}

impl ManagerSession {
    pub fn write(&self) -> Result<()> {
        fs::create_dir_all(&self.dir).with_context(|| format!("failed to create {}", self.dir))?;
        fs::write(&self.meta.brief, &self.brief)
            .with_context(|| format!("failed to write {}", self.meta.brief))?;
        write_session_meta(&self.meta.workspace, &self.meta)
    }
}

pub(super) fn write_session_meta(workspace: &Utf8Path, meta: &SessionMeta) -> Result<()> {
    let meta_path = session_meta_path(workspace, &meta.id)?;
    crate::store::write_json(&meta_path, meta)
}

fn render_entry_brief(
    agent: &agents::AgentSpec,
    workspace: &Utf8Path,
    dir: &Utf8Path,
    startup_context: &str,
    template: &str,
) -> Result<String> {
    let manifest_path = workspace_manifest::manifest_path(workspace)?;
    Ok(render_template(
        template,
        &[
            ("{workspace}", workspace.as_str()),
            ("{agent}", &agent.canonical()),
            ("{dir}", dir.as_str()),
            ("{manifest}", manifest_path.as_str()),
            ("{startup_context}", startup_context),
        ],
    ))
}

fn session_meta_path(workspace: &Utf8Path, id: &str) -> Result<Utf8PathBuf> {
    Ok(sessions_dir(workspace)?.join(id).join("session.json"))
}

pub(crate) fn live_lead(workspace: &Utf8Path) -> Result<Option<SessionMeta>> {
    let Some(pane) = recorded_lead_pane() else {
        return Ok(None);
    };
    Ok(latest_lead(workspace)?.filter(|meta| meta.lead_pane.as_deref() == Some(&pane)))
}

pub(crate) fn latest_lead(workspace: &Utf8Path) -> Result<Option<SessionMeta>> {
    for dir in read_dir_utf8_paths(&sessions_dir(workspace)?)?
        .into_iter()
        .rev()
    {
        if dir.is_dir()
            && let Some(meta) = read_session_meta(&dir)?
        {
            return Ok(Some(meta));
        }
    }
    Ok(None)
}

pub(crate) fn read_session_meta(dir: &Utf8Path) -> Result<Option<SessionMeta>> {
    let meta: Option<SessionMeta> = crate::store::read_optional_json(&dir.join("session.json"))?;
    if let Some(meta) = &meta {
        ensure!(
            Some(meta.id.as_str()) == dir.file_name(),
            "session id does not match directory {dir}"
        );
    }
    Ok(meta)
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
    static LEAD_BRIEF_TEMPLATE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let root = temp_test_path("lead-body");
        crate::skills::resolve(&SkillName::default(), &root, &root)
            .unwrap()
            .body
    });

    #[test]
    fn lead_brief_bounds_the_lead_to_the_request() {
        for phrase in [
            "The operator's request sets the scope of what you do",
            "is not commissioned again",
            "mean the named object only",
            "only when the operator says so",
        ] {
            assert!(LEAD_BRIEF_TEMPLATE.contains(phrase), "missing {phrase:?}");
        }
    }

    /// Designing, estimating and judging security belong to the designers now.
    #[test]
    fn lead_brief_coordinates() {
        assert!(LEAD_BRIEF_TEMPLATE.contains("You coordinate"));
        for moved in [
            "economy pass",
            "race accepted",
            "outbound calls built from untrusted data",
            "worker_planning",
            "Lead decision",
            "{review_instruction}",
        ] {
            assert!(
                !LEAD_BRIEF_TEMPLATE.contains(moved),
                "lead brief still carries {moved:?}"
            );
        }
    }

    #[test]
    fn lead_brief_routes_tiers_with_operator_approval_without_hardcoding_models() {
        for rule in [
            "Default to the standard primary",
            "`niles models`",
            "reason and cost difference",
            "then wait for the operator's yes",
            "Propose stepping down when economical and appropriate",
        ] {
            assert!(LEAD_BRIEF_TEMPLATE.contains(rule), "missing {rule:?}");
        }
        for model in ["opus", "sonnet", "gpt-5", "claude:", "codex:gpt"] {
            assert!(
                !LEAD_BRIEF_TEMPLATE.contains(model),
                "lead brief should not hardcode the model ladder, found {model:?}"
            );
        }
    }

    #[test]
    fn lead_brief_does_not_duplicate_fetched_commands() {
        for command in [
            "niles peek <id>",
            "niles close <id>",
            "niles wait <id>",
            "niles wait --all",
        ] {
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
            entry_skill: SkillName::default(),
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
        let file = session_meta_path(&workspace, &meta.id).unwrap();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        write_session_meta(&workspace, &meta).unwrap();

        let body = fs::read_to_string(&file).unwrap();
        assert!(body.contains("\"lead_pane\": \"%7\""), "{body}");

        // A session with no pane omits the field rather than writing a null nobody records.
        meta.lead_pane = None;
        write_session_meta(&workspace, &meta).unwrap();
        let body = fs::read_to_string(&file).unwrap();
        assert!(!body.contains("lead_pane"), "{body}");

        crate::test_support::remove_workspace(&workspace).unwrap();
    }

    #[test]
    fn lead_brief_render_fills_every_placeholder() {
        let workspace = temp_test_path("brief-{dir}-render");
        let dir = workspace.join(".niles/sessions/test-session");
        let models = agents::ModelRoster::builtin().unwrap();
        let agent = agents::AgentSpec::parse("codex:gpt-5.5:xhigh", &models).unwrap();

        let body = render_entry_brief(
            &agent,
            &workspace,
            &dir,
            "worker: none",
            &LEAD_BRIEF_TEMPLATE,
        )
        .unwrap();

        assert!(body.contains(&format!(
            "manifest: {}",
            workspace_manifest::manifest_path(&workspace).unwrap()
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
