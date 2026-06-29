use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::{crew, session, store, util::write_json_pretty};

const WAKE_DIR: &str = ".niles/wake";

#[derive(Debug, Default, Serialize, Deserialize)]
struct WakeState {
    seen_lines: BTreeMap<String, usize>,
}

/// A status log to drain, plus the command that inspects its source.
struct WakeTarget {
    id: String,
    status: Utf8PathBuf,
    inspect: String,
}

struct Wake {
    id: String,
    state: String,
    message: String,
    inspect: String,
}

pub fn watch_crew(session_id: Option<String>, interval: f64, once: bool) -> Result<()> {
    if !interval.is_finite() || interval <= 0.0 {
        bail!("watch-crew interval must be a finite positive number");
    }

    loop {
        drain_once(session_id.as_deref())?;
        if once {
            break;
        }
        thread::sleep(Duration::from_secs_f64(interval));
    }

    Ok(())
}

fn drain_once(session_id: Option<&str>) -> Result<()> {
    let mut state = read_state()?;
    let mut wakes = Vec::new();

    for target in wake_targets(&state.seen_lines)? {
        let body = match fs::read_to_string(&target.status) {
            Ok(body) => body,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", target.status));
            }
        };
        let lines = body.lines().collect::<Vec<_>>();
        let start = state
            .seen_lines
            .get(&target.id)
            .copied()
            .unwrap_or_default()
            .min(lines.len());

        for line in &lines[start..] {
            if let Some((wake_state, message)) = parse_wake(line) {
                wakes.push(Wake {
                    id: target.id.clone(),
                    state: wake_state,
                    message,
                    inspect: target.inspect.clone(),
                });
            }
        }

        state.seen_lines.insert(target.id, lines.len());
    }

    write_state(&state)?;

    for wake in wakes {
        dispatch_wake(session_id, &wake)?;
    }

    Ok(())
}

/// Status logs the watcher drains: crew worker panes plus run steps. A run that
/// has already finished is included only when the watcher was tracking it (its
/// id is in `seen`), so a continuously-running watcher delivers a run's final
/// wake without replaying old runs it cold-starts against.
fn wake_targets(seen: &BTreeMap<String, usize>) -> Result<Vec<WakeTarget>> {
    let mut targets = Vec::new();
    for target in crew::status_targets()? {
        let inspect = format!("niles peek {}", target.id);
        targets.push(WakeTarget {
            id: target.id,
            status: target.status,
            inspect,
        });
    }
    for target in store::run_status_targets()? {
        if target.terminal && !seen.contains_key(&target.id) {
            continue;
        }
        let inspect = format!("niles status {}", target.id);
        targets.push(WakeTarget {
            id: target.id,
            status: target.status,
            inspect,
        });
    }
    Ok(targets)
}

fn parse_wake(line: &str) -> Option<(String, String)> {
    let (state, message) = line.split_once(':')?;
    let state = state.trim();
    if !matches!(state, "done" | "failed" | "blocked" | "needs-decision") {
        return None;
    }
    Some((state.to_owned(), message.trim().to_owned()))
}

fn dispatch_wake(session_id: Option<&str>, wake: &Wake) -> Result<()> {
    let line = format!(
        "Niles wake: {} {}: {}. Inspect with `{}`.",
        wake.id, wake.state, wake.message, wake.inspect
    );
    append_queue(&line)?;

    let Some(meta) = session::read_session_meta(session_id)? else {
        println!("{line}");
        return Ok(());
    };
    let Some(target) = meta.supervisor_target else {
        println!("{line}");
        return Ok(());
    };

    send_to_tmux(&target, &line)
}

fn send_to_tmux(target: &str, line: &str) -> Result<()> {
    run_tmux(["send-keys", "-t", target, "-l", line])?;
    run_tmux(["send-keys", "-t", target, "Enter"])
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

fn append_queue(line: &str) -> Result<()> {
    let path = Utf8Path::new(WAKE_DIR).join("queue.log");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    }
    let mut body = line.to_owned();
    body.push('\n');
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open {path}"))?
        .write_all(body.as_bytes())
        .with_context(|| format!("failed to write {path}"))
}

fn read_state() -> Result<WakeState> {
    let path = state_path();
    match fs::read_to_string(&path) {
        Ok(body) => serde_json::from_str(&body).with_context(|| format!("failed to parse {path}")),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(WakeState::default()),
        Err(err) => Err(err).with_context(|| format!("failed to read {path}")),
    }
}

fn write_state(state: &WakeState) -> Result<()> {
    let path = state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    }
    write_json_pretty(&path, state)
}

fn state_path() -> Utf8PathBuf {
    Utf8Path::new(WAKE_DIR).join("state.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_actionable_wakes() {
        let (state, message) = parse_wake("done: tests pass").unwrap();

        assert_eq!(state, "done");
        assert_eq!(message, "tests pass");
        assert!(parse_wake("working: running tests").is_none());
    }
}
