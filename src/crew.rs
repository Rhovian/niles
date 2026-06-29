use std::fs;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::{
    config::{
        agents,
        spec::{PromptMode, load_project_config_from},
    },
    tmux,
    util::{
        absolute_existing_dir, absolute_existing_file, absolute_path, utf8_path, write_json_pretty,
    },
};

const CREW_DIR: &str = ".niles/crew";

#[derive(Debug, Serialize, Deserialize)]
struct CrewMeta {
    id: String,
    agent: String,
    project: Utf8PathBuf,
    window: String,
    brief: Utf8PathBuf,
    launch: Utf8PathBuf,
    status: Option<Utf8PathBuf>,
}

pub struct CrewStatusTarget {
    pub id: String,
    pub status: Utf8PathBuf,
}

pub fn spawn(
    id: String,
    project: Utf8PathBuf,
    agent: String,
    brief: Option<Utf8PathBuf>,
    task: Vec<String>,
) -> Result<()> {
    validate_id(&id)?;
    if brief.is_none() && task.is_empty() {
        bail!("spawn requires either --brief or task text");
    }

    let project = absolute_existing_dir(&project, "project")?;
    let dir = absolute_path(Utf8Path::new(CREW_DIR))?.join(&id);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;

    let brief_path = match brief {
        Some(path) => absolute_existing_file(&path, "brief")?,
        None => {
            let path = dir.join("brief.md");
            write_brief(&path, &id, &project, &agent, &task.join(" "))?;
            path
        }
    };

    let launch_path = dir.join("launch.sh");
    let status_path = dir.join("status.log");
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&status_path)
        .with_context(|| format!("failed to create {status_path}"))?;

    let window_name = format!("niles-{id}");
    let target = spawn_agent_window(
        &window_name,
        &project,
        &agent,
        &project,
        &brief_path,
        &launch_path,
    )?;

    let meta = CrewMeta {
        id: id.clone(),
        agent,
        project,
        window: target.clone(),
        brief: brief_path,
        launch: launch_path,
        status: Some(status_path),
    };
    write_meta(&id, &meta)?;

    println!("spawned: {id}");
    println!("window: {target}");
    println!("agent: {}", meta.agent);
    println!("brief: {}", meta.brief);
    println!("peek: niles peek {id}");
    println!("send: niles send {id} <message>");

    Ok(())
}

pub fn peek(id: String, lines: usize) -> Result<()> {
    let meta = read_meta(&id)?;
    print!("{}", capture_pane(&meta.window, lines)?);
    Ok(())
}

/// Capture the last `lines` of a step window's pane by window name, resolving
/// the active session. Used to fold an interactive step's output into the run.
pub(crate) fn capture_window(window_name: &str, lines: usize) -> Result<String> {
    let session = tmux::current_or_named_session("niles")?;
    capture_pane(&format!("{session}:{window_name}"), lines)
}

fn capture_pane(target: &str, lines: usize) -> Result<String> {
    tmux::capture_pane(target, lines)
}

pub fn send(id: String, message: Vec<String>) -> Result<()> {
    if message.is_empty() {
        bail!("send requires a message");
    }

    let meta = read_meta(&id)?;
    let message = message.join(" ");
    tmux::send_line(&meta.window, &message)?;
    println!("sent: {id}");
    Ok(())
}

pub fn status_targets() -> Result<Vec<CrewStatusTarget>> {
    let dir = Utf8Path::new(CREW_DIR);
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("failed to read {dir}")),
    };

    let mut targets = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("failed to read entry in {dir}"))?;
        let path = utf8_path(entry.path(), "crew metadata path")?;
        if path.extension() != Some("json") {
            continue;
        }
        let body = fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
        let meta = serde_json::from_str::<CrewMeta>(&body)
            .with_context(|| format!("failed to parse {path}"))?;
        if let Some(status) = meta.status {
            targets.push(CrewStatusTarget {
                id: meta.id,
                status,
            });
        }
    }

    targets.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(targets)
}

fn write_brief(
    path: &Utf8Path,
    id: &str,
    project: &Utf8Path,
    agent: &str,
    task: &str,
) -> Result<()> {
    let status_path = path
        .parent()
        .map(|parent| parent.join("status.log"))
        .unwrap_or_else(|| Utf8PathBuf::from("status.log"));
    let body = format!(
        "# Niles Crew Brief\n\nid: {id}\nproject: {project}\nagent: {agent}\nstatus_file: {status_path}\n\n## Task\n\n{task}\n\n## Operating Notes\n\nWork autonomously in this tmux window. Report concise status and final results in your terminal output. The foreground Niles supervisor can inspect this pane with `niles peek {id}` and steer it with `niles send {id} <message>`.\n\n## Wake Contract\n\nAppend actionable status lines to the status file so Niles can wake the foreground supervisor:\n\n```sh\necho \"done: short result\" >> {status_path}\necho \"blocked: blocker summary\" >> {status_path}\necho \"needs-decision: decision needed\" >> {status_path}\necho \"failed: failure summary\" >> {status_path}\n```\n\nUse `working:` sparingly for durable phase changes; it is recorded but does not wake the supervisor.\n"
    );
    fs::write(path, body).with_context(|| format!("failed to write {path}"))
}

fn write_launch_script(
    path: &Utf8Path,
    invocation: &agents::AgentInvocation,
    brief_path: &Utf8Path,
) -> Result<()> {
    let mut body = String::new();
    body.push_str("#!/bin/sh\n");
    body.push_str("set -eu\n");
    body.push_str("BRIEF=");
    body.push_str(&shell_quote(brief_path.as_str()));
    body.push('\n');
    if invocation.binary == "claude" {
        body.push_str("export CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=false\n");
    }
    body.push_str("exec ");
    body.push_str(&shell_quote(&invocation.binary));
    for arg in &invocation.args {
        body.push(' ');
        body.push_str(&shell_quote(arg));
    }
    match invocation.prompt {
        PromptMode::Arg => body.push_str(" \"$(cat \"$BRIEF\")\"\n"),
        PromptMode::Stdin => body.push_str(" < \"$BRIEF\"\n"),
    }

    fs::write(path, body).with_context(|| format!("failed to write {path}"))
}

/// Launch `agent` interactively in a fresh tmux window driven by `brief_path`,
/// returning its `session:window` target. Shared by `spawn` and per-step
/// orchestration so both use the same interactive invocation and launch script.
pub(crate) fn spawn_agent_window(
    window_name: &str,
    cwd: &Utf8Path,
    agent: &str,
    project: &Utf8Path,
    brief_path: &Utf8Path,
    launch_path: &Utf8Path,
) -> Result<String> {
    let config = load_project_config_from(project)?;
    let invocation = agents::invocation(
        agent,
        config.agents.get(agent),
        agents::InvocationDefaults::Worker,
    );
    write_launch_script(launch_path, &invocation, brief_path)?;
    let command = format!("sh {}", shell_quote(launch_path.as_str()));
    open_window(window_name, cwd, &command)
}

/// Kill a tmux window by name in the active session. Used to tear down a step
/// window once the supervisor decides the step is done.
pub(crate) fn close_window(window_name: &str) -> Result<()> {
    let session = tmux::current_or_named_session("niles")?;
    tmux::kill_window(&session, window_name)
}

/// Open a detached tmux window running `command` in `cwd` and return its
/// `session:window` target. Shared by `spawn` and the per-step orchestrator.
pub(crate) fn open_window(window_name: &str, cwd: &Utf8Path, command: &str) -> Result<String> {
    let session = tmux::current_or_named_session("niles")?;
    tmux::ensure_window_available(&session, window_name)?;
    let target = format!("{session}:{window_name}");
    tmux::new_window(&session, window_name, cwd, command)?;
    Ok(target)
}

fn write_meta(id: &str, meta: &CrewMeta) -> Result<()> {
    let path = meta_path(id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    }
    write_json_pretty(&path, meta)
}

fn read_meta(id: &str) -> Result<CrewMeta> {
    validate_id(id)?;
    let path = meta_path(id);
    let body = fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
    serde_json::from_str(&body).with_context(|| format!("failed to parse {path}"))
}

fn meta_path(id: &str) -> Utf8PathBuf {
    Utf8Path::new(CREW_DIR).join(format!("{id}.json"))
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() {
        bail!("crew id cannot be empty");
    }
    if !id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("crew id may only contain ASCII letters, numbers, '-' and '_'");
    }
    Ok(())
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quotes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
