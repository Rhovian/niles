use std::{fs, io::ErrorKind};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::{
    config::{
        agents,
        spec::{PromptMode, load_project_config_from},
        version,
    },
    store::{read_state, resolve_run_dir},
    tmux,
    util::{absolute_existing_dir, absolute_existing_file, absolute_path, write_json_pretty},
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

enum PaneTarget {
    Crew {
        id: String,
        target: String,
    },
    RunStep {
        run: String,
        index: usize,
        window_name: String,
    },
}

pub fn spawn(
    id: String,
    project: Utf8PathBuf,
    agent: String,
    brief: Option<Utf8PathBuf>,
    task: Vec<String>,
    allow_cli_mismatch: bool,
) -> Result<()> {
    validate_id(&id)?;
    if brief.is_none() && task.is_empty() {
        bail!("spawn requires either --brief or task text");
    }

    let project = absolute_existing_dir(&project, "project")?;
    let config = load_project_config_from(&project)?;
    version::preflight_agent(
        &agent,
        config.agents.get(&agent),
        agents::InvocationDefaults::Worker,
        allow_cli_mismatch,
    )?;
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
    println!("window: {window_name}");
    println!("agent: {}", meta.agent);
    println!("brief: {}", meta.brief);
    println!("peek: niles peek {id}");
    println!("send: niles send {id} <message>");
    println!("close: niles crew-close {id}");

    Ok(())
}

/// Tear down a spawned crew worker. The tmux window may already be gone, so
/// window close errors are reported but do not strand metadata.
pub fn crew_close(id: String) -> Result<()> {
    validate_id(&id)?;
    let crew_dir = Utf8Path::new(CREW_DIR).join(&id);
    let meta = read_meta_if_exists(&id)?;
    if meta.is_none() && !crew_dir.exists() {
        bail!("unknown crew id '{id}'");
    }
    let window_name = meta
        .as_ref()
        .map(|meta| crew_window_name(&id, meta))
        .unwrap_or_else(|| format!("niles-{id}"));

    match close_window(&window_name) {
        Ok(()) => println!("closed window: {window_name}"),
        Err(err) => println!("window {window_name} not closed: {err}"),
    }

    remove_file_if_exists(&meta_path(&id))?;
    remove_dir_all_if_exists(&crew_dir)?;
    println!("closed: {id}");
    Ok(())
}

pub fn peek(
    id: Option<String>,
    run: Option<String>,
    index: Option<usize>,
    lines: usize,
) -> Result<()> {
    let target = resolve_peek_target(id, run, index)?;
    print!("{}", target.capture(lines)?);
    Ok(())
}

/// Capture the last `lines` of a step window's pane by window name, resolving
/// the active session. Used to fold an interactive step's output into the run.
pub(crate) fn capture_window(window_name: &str, lines: usize) -> Result<String> {
    capture_pane(&active_window_target(window_name)?, lines)
}

fn capture_pane(target: &str, lines: usize) -> Result<String> {
    tmux::capture_pane(target, lines)
}

pub fn send(
    run: Option<String>,
    index: Option<usize>,
    target_and_message: Vec<String>,
) -> Result<()> {
    if target_and_message.is_empty() {
        bail!("send requires a message");
    }

    let (target, message) = resolve_send_target(run, index, target_and_message)?;
    let message = message.join(" ");
    target.send(&message)?;
    println!("sent: {}", target.label());
    Ok(())
}

impl PaneTarget {
    fn capture(&self, lines: usize) -> Result<String> {
        match self {
            PaneTarget::Crew { target, .. } => capture_pane(target, lines),
            PaneTarget::RunStep { window_name, .. } => capture_window(window_name, lines),
        }
    }

    fn send(&self, message: &str) -> Result<()> {
        match self {
            PaneTarget::Crew { target, .. } => tmux::send_line(target, message),
            PaneTarget::RunStep { window_name, .. } => {
                tmux::send_line(&active_window_target(window_name)?, message)
            }
        }
    }

    fn label(&self) -> String {
        match self {
            PaneTarget::Crew { id, .. } => id.clone(),
            PaneTarget::RunStep { run, index, .. } => format!("{run} step {index}"),
        }
    }
}

fn resolve_peek_target(
    id: Option<String>,
    run: Option<String>,
    index: Option<usize>,
) -> Result<PaneTarget> {
    let has_step_target = run.is_some() || index.is_some();
    match (id, has_step_target) {
        (Some(_), true) => bail!("use either a crew id or --run <id> --index <N>, not both"),
        (Some(id), false) => crew_target(id),
        (None, true) => run_step_target(run, index),
        (None, false) => bail!("peek requires a crew id or --run <id> --index <N>"),
    }
}

fn resolve_send_target(
    run: Option<String>,
    index: Option<usize>,
    target_and_message: Vec<String>,
) -> Result<(PaneTarget, Vec<String>)> {
    if run.is_some() || index.is_some() {
        return Ok((run_step_target(run, index)?, target_and_message));
    }

    let mut parts = target_and_message.into_iter();
    let id = parts
        .next()
        .context("send requires a crew id or --run <id> --index <N>")?;
    let message = parts.collect::<Vec<_>>();
    if message.is_empty() {
        bail!("send requires a message");
    }
    Ok((crew_target(id)?, message))
}

fn crew_target(id: String) -> Result<PaneTarget> {
    let meta = read_meta(&id)?;
    Ok(PaneTarget::Crew {
        id,
        target: meta.window,
    })
}

fn run_step_target(run: Option<String>, index: Option<usize>) -> Result<PaneTarget> {
    let run = run.context("run-step target requires --run <id>")?;
    let index = index.context("run-step target requires --index <N>")?;
    if index == 0 {
        bail!("step index must be >= 1");
    }

    let run_dir = resolve_run_dir(&run)?;
    let state = read_state(&run_dir)?;
    let run_id = state.id.clone();
    let step = state
        .steps
        .iter()
        .find(|step| step.index == index)
        .with_context(|| format!("step {index} not found in run {run_id}"))?;
    let window_name = step
        .window
        .clone()
        .with_context(|| format!("step {index} in run {run_id} has no recorded window"))?;

    Ok(PaneTarget::RunStep {
        run: run_id,
        index,
        window_name,
    })
}

fn active_window_target(window_name: &str) -> Result<String> {
    let session = tmux::current_or_named_session("niles")?;
    Ok(format!("{session}:{window_name}"))
}

pub fn status_log_path(id: &str) -> Result<Utf8PathBuf> {
    validate_id(id)?;
    if let Some(meta) = read_meta_if_exists(id)?
        && let Some(status) = meta.status
    {
        return Ok(status);
    }
    Ok(Utf8Path::new(CREW_DIR).join(id).join("status.log"))
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
    read_meta_if_exists(id)?.with_context(|| format!("unknown crew id '{id}'"))
}

fn read_meta_if_exists(id: &str) -> Result<Option<CrewMeta>> {
    let path = meta_path(id);
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to read {path}")),
    };
    serde_json::from_str(&body)
        .map(Some)
        .with_context(|| format!("failed to parse {path}"))
}

fn meta_path(id: &str) -> Utf8PathBuf {
    Utf8Path::new(CREW_DIR).join(format!("{id}.json"))
}

fn crew_window_name(id: &str, meta: &CrewMeta) -> String {
    meta.window
        .rsplit_once(':')
        .map(|(_, window)| window.to_owned())
        .unwrap_or_else(|| format!("niles-{id}"))
}

fn remove_file_if_exists(path: &Utf8Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to remove {path}")),
    }
}

fn remove_dir_all_if_exists(path: &Utf8Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to remove {path}")),
    }
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
