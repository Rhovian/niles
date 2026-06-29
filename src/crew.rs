use std::{
    env, fs,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::{
    config::{
        agents,
        spec::{AgentConfig, PromptMode, load_project_config_from},
    },
    util::write_json_pretty,
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

struct AgentInvocation {
    binary: String,
    args: Vec<String>,
    prompt: PromptMode,
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

    let project = absolute_existing_dir(&project)?;
    let dir = absolute_path(Utf8Path::new(CREW_DIR))?.join(&id);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;

    let brief_path = match brief {
        Some(path) => absolute_existing_file(&path)?,
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
    let target = spawn_agent_window(&window_name, &project, &agent, &project, &brief_path, &launch_path)?;

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
    let session = tmux_session()?;
    capture_pane(&format!("{session}:{window_name}"), lines)
}

fn capture_pane(target: &str, lines: usize) -> Result<String> {
    let output = Command::new("tmux")
        .args(["capture-pane", "-p", "-t", target, "-S", &format!("-{lines}")])
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run tmux capture-pane for {target}"))?;

    if !output.status.success() {
        bail!(
            "tmux capture-pane failed for {target}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    // tmux pads the capture to the pane height; drop the trailing blank lines.
    let text = String::from_utf8_lossy(&output.stdout);
    let trimmed = text.trim_end();
    Ok(if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    })
}

pub fn send(id: String, message: Vec<String>) -> Result<()> {
    if message.is_empty() {
        bail!("send requires a message");
    }

    let meta = read_meta(&id)?;
    let message = message.join(" ");
    run_tmux(["send-keys", "-t", &meta.window, "-l", &message])?;
    run_tmux(["send-keys", "-t", &meta.window, "Enter"])?;
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
        let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|path| {
            anyhow::anyhow!("crew metadata path is not UTF-8: {}", path.display())
        })?;
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

fn agent_invocation(agent: &str, config: Option<&AgentConfig>) -> AgentInvocation {
    match config {
        Some(config) => AgentInvocation {
            binary: config
                .binary
                .clone()
                .unwrap_or_else(|| agents::worker_binary(agent)),
            args: if config.args.is_empty() {
                agents::worker_args(agent)
            } else {
                config.args.clone()
            },
            prompt: config.prompt,
        },
        None => AgentInvocation {
            binary: agents::worker_binary(agent),
            args: agents::worker_args(agent),
            prompt: agents::worker_prompt(agent),
        },
    }
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
    invocation: &AgentInvocation,
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
    let invocation = agent_invocation(agent, config.agents.get(agent));
    write_launch_script(launch_path, &invocation, brief_path)?;
    let command = format!("sh {}", shell_quote(launch_path.as_str()));
    open_window(window_name, cwd, &command)
}

/// Kill a tmux window by name in the active session. Used to tear down a step
/// window once the supervisor decides the step is done.
pub(crate) fn close_window(window_name: &str) -> Result<()> {
    let session = tmux_session()?;
    run_tmux(["kill-window", "-t", &format!("{session}:{window_name}")])
}

/// Open a detached tmux window running `command` in `cwd` and return its
/// `session:window` target. Shared by `spawn` and the per-step orchestrator.
pub(crate) fn open_window(window_name: &str, cwd: &Utf8Path, command: &str) -> Result<String> {
    let session = tmux_session()?;
    ensure_window_available(&session, window_name)?;
    let target = format!("{session}:{window_name}");
    run_tmux([
        "new-window",
        "-d",
        "-t",
        &session,
        "-n",
        window_name,
        "-c",
        cwd.as_str(),
        command,
    ])?;
    Ok(target)
}

fn tmux_session() -> Result<String> {
    if env::var_os("TMUX").is_some() {
        let output = Command::new("tmux")
            .args(["display-message", "-p", "#S"])
            .stdin(Stdio::null())
            .output()
            .context("failed to query current tmux session")?;
        if output.status.success() {
            let session = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !session.is_empty() {
                return Ok(session);
            }
        }
    }

    match Command::new("tmux")
        .args(["has-session", "-t", "niles"])
        .stdin(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok("niles".to_owned()),
        _ => {
            run_tmux(["new-session", "-d", "-s", "niles"])?;
            Ok("niles".to_owned())
        }
    }
}

fn ensure_window_available(session: &str, window_name: &str) -> Result<()> {
    let output = Command::new("tmux")
        .args(["list-windows", "-t", session, "-F", "#{window_name}"])
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to list tmux windows in session {session}"))?;

    if !output.status.success() {
        bail!(
            "tmux list-windows failed for session {session}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let exists = String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line == window_name);
    if exists {
        bail!("tmux window {session}:{window_name} already exists");
    }

    Ok(())
}

fn run_tmux<I, S>(args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect::<Vec<_>>();
    let status = Command::new("tmux")
        .args(&args)
        .stdin(Stdio::null())
        .status()
        .with_context(|| format!("failed to run tmux {}", args.join(" ")))?;
    if !status.success() {
        bail!("tmux {} exited with {status}", args.join(" "));
    }
    Ok(())
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

fn absolute_existing_dir(path: &Utf8Path) -> Result<Utf8PathBuf> {
    let path = absolute_path(path)?;
    if !path.is_dir() {
        bail!("project path is not a directory: {path}");
    }
    Ok(path)
}

fn absolute_existing_file(path: &Utf8Path) -> Result<Utf8PathBuf> {
    let path = absolute_path(path)?;
    if !path.is_file() {
        bail!("brief path is not a file: {path}");
    }
    Ok(path)
}

fn absolute_path(path: &Utf8Path) -> Result<Utf8PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    let cwd = env::current_dir().context("failed to read current directory")?;
    let cwd = Utf8PathBuf::from_path_buf(cwd)
        .map_err(|path| anyhow::anyhow!("current directory is not UTF-8: {}", path.display()))?;
    Ok(cwd.join(path))
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
