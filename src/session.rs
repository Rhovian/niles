use std::{
    env, fs,
    process::{Child, Command, Stdio},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    config::agents,
    util::{timestamp_id, write_json_pretty},
};

const SUPERVISOR_BRIEF_TEMPLATE: &str = include_str!("templates/supervisor_brief.md");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub agent: String,
    pub workspace: Utf8PathBuf,
    pub supervisor_target: Option<String>,
    pub brief: Utf8PathBuf,
}

pub fn run(agent: String, goal: Option<String>) -> Result<()> {
    launch_foreground_agent(&agent, goal.as_deref())
}

fn launch_foreground_agent(agent: &str, goal: Option<&str>) -> Result<()> {
    let binary = agents::foreground_binary(agent);
    let mut args = agents::foreground_args(agent);
    let meta = write_supervisor_session(agent, goal)?;
    let brief = fs::read_to_string(&meta.brief)
        .with_context(|| format!("failed to read supervisor brief {}", meta.brief))?;
    args.extend(supervisor_prompt_args(agent, brief, goal));
    let mut watcher = start_wake_watcher(&meta)?;

    let status = Command::new(&binary)
        .args(&args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to launch foreground agent `{binary}`"))?;
    stop_wake_watcher(&mut watcher);

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
    let workspace = current_workspace()?;
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
        supervisor_target: supervisor_target(),
        brief: path,
    };
    let meta_path = session_meta_path(&id);
    write_json_pretty(&meta_path, &meta)?;
    fs::write(latest_session_path(), &id).context("failed to write latest session pointer")?;
    Ok(meta)
}

pub fn read_session_meta(session: Option<&str>) -> Result<Option<SessionMeta>> {
    let id = match session {
        Some(session) => session.to_owned(),
        None => match fs::read_to_string(latest_session_path()) {
            Ok(id) => id.trim().to_owned(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).context("failed to read latest session pointer"),
        },
    };
    if id.is_empty() {
        return Ok(None);
    }

    let path = session_meta_path(&id);
    let body = fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
    serde_json::from_str(&body)
        .map(Some)
        .with_context(|| format!("failed to parse {path}"))
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

fn supervisor_target() -> Option<String> {
    env::var("TMUX_PANE")
        .ok()
        .filter(|target| !target.trim().is_empty())
}

fn start_wake_watcher(meta: &SessionMeta) -> Result<Option<Child>> {
    if meta.supervisor_target.is_none() || env::var_os("NILES_NO_WATCHER").is_some() {
        return Ok(None);
    }

    let exe = env::current_exe().context("failed to resolve niles executable")?;
    let child = Command::new(exe)
        .args(["watch-crew", "--session", &meta.id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to start crew wake watcher")?;
    Ok(Some(child))
}

fn stop_wake_watcher(watcher: &mut Option<Child>) {
    if let Some(child) = watcher {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn current_workspace() -> Result<Utf8PathBuf> {
    let cwd = env::current_dir().context("failed to read current directory")?;
    Utf8PathBuf::from_path_buf(cwd)
        .map_err(|path| anyhow::anyhow!("current directory is not UTF-8: {}", path.display()))
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
