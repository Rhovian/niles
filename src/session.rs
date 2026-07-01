use std::{
    env,
    ffi::OsStr,
    fs,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    config::agents,
    util::{current_dir_utf8, timestamp_id, write_json_pretty},
    workspace_manifest::{self, WorkspaceManifest, WorkspaceManifestDefaults},
};

const SUPERVISOR_BRIEF_TEMPLATE: &str = include_str!("templates/supervisor_brief.md");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub agent: String,
    pub workspace: Utf8PathBuf,
    pub brief: Utf8PathBuf,
}

pub fn run(supervisor: Option<String>, goal: Option<String>) -> Result<()> {
    let manifest = launch_prelude(supervisor.as_deref())?;
    launch_foreground_agent(&manifest.supervisor, goal.as_deref())
}

fn launch_prelude(supervisor_override: Option<&str>) -> Result<WorkspaceManifest> {
    require_tmux_session(env::var_os("TMUX").as_deref())?;

    let mut defaults = WorkspaceManifestDefaults::default();
    if let Some(supervisor) = supervisor_override {
        defaults.supervisor = supervisor.to_owned();
    }

    let root = Utf8Path::new(".");
    workspace_manifest::ensure_interactive(root, &defaults)
}

fn require_tmux_session(tmux: Option<&OsStr>) -> Result<()> {
    if tmux_session_present(tmux) {
        return Ok(());
    }

    bail!(
        "Niles must be launched from inside a tmux session. Start one with `tmux new -s niles` or attach with `tmux attach`, then run `niles` again."
    )
}

fn tmux_session_present(tmux: Option<&OsStr>) -> bool {
    tmux.and_then(OsStr::to_str)
        .is_some_and(|value| !value.trim().is_empty())
}

fn launch_foreground_agent(agent: &str, goal: Option<&str>) -> Result<()> {
    let binary = agents::foreground_binary(agent);
    let mut args = agents::foreground_args(agent);
    let meta = write_supervisor_session(agent, goal)?;
    let brief = fs::read_to_string(&meta.brief)
        .with_context(|| format!("failed to read supervisor brief {}", meta.brief))?;
    args.extend(supervisor_prompt_args(agent, brief, goal));

    let status = Command::new(&binary)
        .args(&args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to launch foreground agent `{binary}`"))?;

    if status.success() {
        Ok(())
    } else {
        bail!(
            "foreground agent `{agent}` exited with {}",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_owned())
        )
    }
}

fn supervisor_prompt_args(agent: &str, brief: String, goal: Option<&str>) -> Vec<String> {
    let startup_prompt = startup_prompt(goal);
    match agent {
        "claude" => vec!["--append-system-prompt".to_owned(), brief, startup_prompt],
        _ => vec![format!("{brief}\n\n{startup_prompt}")],
    }
}

fn startup_prompt(_goal: Option<&str>) -> String {
    "Start the Niles supervisor session.".to_owned()
}

fn write_supervisor_session(agent: &str, goal: Option<&str>) -> Result<SessionMeta> {
    let workspace = current_dir_utf8()?;
    let now = Utc::now();
    let id = timestamp_id(&now);
    let dir = Utf8Path::new(".niles").join("sessions").join(&id);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;
    let path = dir.join("supervisor.md");
    let goal = goal
        .unwrap_or("No initial goal was provided. Start by asking the user what they want done.");
    let startup_context = startup_context()?;
    let body = SUPERVISOR_BRIEF_TEMPLATE
        .replace("{workspace}", workspace.as_str())
        .replace("{agent}", agent)
        .replace("{dir}", dir.as_str())
        .replace("{goal}", goal)
        .replace("{startup_context}", &startup_context);
    fs::write(&path, body).with_context(|| format!("failed to write {path}"))?;
    let meta = SessionMeta {
        id: id.clone(),
        agent: agent.to_owned(),
        workspace,
        brief: path,
    };
    let meta_path = session_meta_path(&id);
    write_json_pretty(&meta_path, &meta)?;
    fs::write(latest_session_path(), &id).context("failed to write latest session pointer")?;
    Ok(meta)
}

fn session_meta_path(id: &str) -> Utf8PathBuf {
    Utf8Path::new(".niles")
        .join("sessions")
        .join(id)
        .join("session.json")
}

fn latest_session_path() -> Utf8PathBuf {
    Utf8Path::new(".niles").join("sessions").join("latest")
}

fn startup_context() -> Result<String> {
    let lines = [latest_run_context()?, crew_context()?];
    Ok(lines.join("\n"))
}

fn latest_run_context() -> Result<String> {
    let runs_dir = Utf8Path::new(".niles").join("runs");
    let mut runs = match fs::read_dir(&runs_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| Utf8PathBuf::from_path_buf(entry.path()).ok())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok("latest_run: none".to_owned());
        }
        Err(err) => return Err(err).with_context(|| format!("failed to read {runs_dir}")),
    };
    runs.sort();
    let Some(run_dir) = runs.pop() else {
        return Ok("latest_run: none".to_owned());
    };

    let state_path = run_dir.join("state.json");
    let body = fs::read_to_string(&state_path)
        .with_context(|| format!("failed to read latest run state {state_path}"))?;
    let value = serde_json::from_str::<serde_json::Value>(&body)
        .with_context(|| format!("failed to parse latest run state {state_path}"))?;
    let id = value
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let status = value
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let goal = value
        .get("goal")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    Ok(format!("latest_run: id={id} status={status} goal={goal:?}"))
}

fn crew_context() -> Result<String> {
    let crew_dir = Utf8Path::new(".niles").join("crew");
    let entries = match fs::read_dir(&crew_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok("crew: none".to_owned());
        }
        Err(err) => return Err(err).with_context(|| format!("failed to read {crew_dir}")),
    };
    let mut ids = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| Utf8PathBuf::from_path_buf(entry.path()).ok())
        .filter(|path| path.extension() == Some("json"))
        .filter_map(|path| {
            path.file_stem()
                .map(|stem| stem.to_owned())
                .filter(|stem| !stem.is_empty())
        })
        .collect::<Vec<_>>();
    ids.sort();
    if ids.is_empty() {
        Ok("crew: none".to_owned())
    } else {
        Ok(format!("crew: {}", ids.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_session_present_accepts_nonempty_env() {
        assert!(tmux_session_present(Some(OsStr::new(
            "/tmp/tmux-501/default,1,0"
        ))));
    }

    #[test]
    fn tmux_session_present_rejects_missing_or_empty_env() {
        assert!(!tmux_session_present(None));
        assert!(!tmux_session_present(Some(OsStr::new(""))));
        assert!(!tmux_session_present(Some(OsStr::new("   "))));
    }

    #[test]
    fn require_tmux_session_errors_when_missing() {
        let err = require_tmux_session(None).unwrap_err();

        assert!(err.to_string().contains("inside a tmux session"));
        assert!(err.to_string().contains("tmux new -s niles"));
    }

    #[test]
    fn supervisor_prompt_args_pass_brief_as_claude_system_prompt() {
        let args = supervisor_prompt_args("claude", "brief body".to_owned(), Some("ship it"));

        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "--append-system-prompt");
        assert_eq!(args[1], "brief body");
        assert_eq!(args[2], "Start the Niles supervisor session.");
    }

    #[test]
    fn supervisor_brief_omits_removed_manifest_command() {
        assert!(SUPERVISOR_BRIEF_TEMPLATE.contains("niles spawn <id>"));
        assert!(SUPERVISOR_BRIEF_TEMPLATE.contains("niles run"));
        assert!(!SUPERVISOR_BRIEF_TEMPLATE.contains("niles manifest"));
    }
}
