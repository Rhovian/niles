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

use crate::{crew, session};

const WAKE_DIR: &str = ".niles/wake";

#[derive(Debug, Default, Serialize, Deserialize)]
struct WakeState {
    seen_lines: BTreeMap<String, usize>,
}

struct Wake {
    crew_id: String,
    state: String,
    message: String,
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

    for target in crew::status_targets()? {
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
            if let Some(wake) = parse_wake(&target.id, line) {
                wakes.push(wake);
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

fn parse_wake(crew_id: &str, line: &str) -> Option<Wake> {
    let (state, message) = line.split_once(':')?;
    let state = state.trim();
    if !matches!(state, "done" | "failed" | "blocked" | "needs-decision") {
        return None;
    }
    let message = message.trim();
    Some(Wake {
        crew_id: crew_id.to_owned(),
        state: state.to_owned(),
        message: message.to_owned(),
    })
}

fn dispatch_wake(session_id: Option<&str>, wake: &Wake) -> Result<()> {
    let line = format!(
        "Niles wake: {} {}: {}. Inspect with `niles peek {}`.",
        wake.crew_id, wake.state, wake.message, wake.crew_id
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
    fs::write(&path, serde_json::to_string_pretty(state)?)
        .with_context(|| format!("failed to write {path}"))
}

fn state_path() -> Utf8PathBuf {
    Utf8Path::new(WAKE_DIR).join("state.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_actionable_wakes() {
        let wake = parse_wake("auth-fix", "done: tests pass").unwrap();

        assert_eq!(wake.crew_id, "auth-fix");
        assert_eq!(wake.state, "done");
        assert_eq!(wake.message, "tests pass");
        assert!(parse_wake("auth-fix", "working: running tests").is_none());
    }
}
