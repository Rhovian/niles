use super::sessions_dir;
use super::startup::startup_context;
use crate::skills::SkillName;
use crate::store::{self, atomic_write};
use crate::telemetry::SessionLink;
use crate::util::{read_dir_utf8_paths, render_template, timestamp_id};
use crate::workspace_manifest::setup::Setup;
use crate::{agents, workspace_manifest};
use anyhow::{Context, Result, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{env, fs};

/// The environment variable tmux sets for the pane a process runs in.
const LEAD_PANE_ENV: &str = "TMUX_PANE";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SessionMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup: Option<Setup>,
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
    source: Option<&workspace_manifest::setup::SetupFile>,
    template: &str,
) -> Result<ManagerSession> {
    let now = Utc::now();
    let id = timestamp_id(&now);
    let dir = sessions_dir(workspace)?.join(&id);
    let path = dir.join("lead.md");
    let startup_context = startup_context(workspace)?;
    let body = render_entry_brief(agent, workspace, &dir, &startup_context, template, source)?;
    let meta = SessionMeta {
        setup: source.map(|file| file.setup.clone()),
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
        session_link: agents::session_link(agent.family(), &dir),
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
        atomic_write(&self.meta.brief, &self.brief)?;
        write_session_meta(&self.meta.workspace, &self.meta)
    }
}

pub(super) fn write_session_meta(workspace: &Utf8Path, meta: &SessionMeta) -> Result<()> {
    let meta_path = session_meta_path(workspace, &meta.id)?;
    store::write_json(&meta_path, meta)
}

fn render_entry_brief(
    agent: &agents::AgentSpec,
    workspace: &Utf8Path,
    dir: &Utf8Path,
    startup_context: &str,
    template: &str,
    source: Option<&workspace_manifest::setup::SetupFile>,
) -> Result<String> {
    let manifest_path = match source {
        Some(file) => file.path().to_owned(),
        None => workspace_manifest::manifest_path(workspace, None)?,
    };
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
    let meta: Option<SessionMeta> = store::read_optional_json(&dir.join("session.json"))?;
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
    use crate::test_support::{self, temp_dir, temp_test_path};

    #[test]
    fn session_meta_records_the_lead_pane() {
        let workspace = temp_dir("session-lead-pane");
        let mut meta = SessionMeta {
            setup: None,
            entry_skill: "lead".parse().unwrap(),
            id: "session".to_owned(),
            agent: "claude".to_owned(),
            agent_family: None,
            model: None,
            effort: None,
            created_at: Utc::now(),
            workspace: workspace.to_path_buf(),
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

        test_support::remove_workspace(&workspace).unwrap();
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
            "{manifest}\n{workspace}\n{agent}\n{dir}\n{startup_context}\n{unknown}",
            None,
        )
        .unwrap();

        assert_eq!(
            body,
            format!(
                "{}\n{workspace}\ncodex:gpt-5.5:xhigh\n{dir}\nworker: none\n{{unknown}}",
                workspace_manifest::manifest_path(&workspace, None).unwrap()
            )
        );
    }
}
