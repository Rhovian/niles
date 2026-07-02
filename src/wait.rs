use std::{
    fs, thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{store, worker};

const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(60 * 60);

pub fn wait(
    run: Option<String>,
    worker: Option<String>,
    index: Option<usize>,
    interval: f64,
    timeout: Option<f64>,
) -> Result<()> {
    if let Some(0) = index {
        bail!("step index must be >= 1");
    }

    let target = match (run, worker) {
        (Some(run), None) => WaitTarget::Run {
            status: store::resolve_run_dir(&run)?.join("status.log"),
        },
        (None, Some(id)) => {
            let status = worker::status_log_path(&id)?;
            let dir = worker_status_dir(&status)?;
            WaitTarget::Worker { id, status, dir }
        }
        (Some(_), Some(_)) => bail!("use either a run id or --worker <id>, not both"),
        (None, None) => bail!("wait requires a run id or --worker <id>"),
    };

    let interval = positive_seconds_duration(interval, "wait interval")?;
    let timeout = timeout
        .map(|seconds| non_negative_seconds_duration(seconds, "wait timeout"))
        .transpose()?
        .unwrap_or(DEFAULT_WAIT_TIMEOUT);

    match wait_for_wake(&target, index, interval, timeout)? {
        WaitResult::Line(line) => {
            println!("{line}");
            Ok(())
        }
        WaitResult::WorkerClosed { id, line } => {
            println!("{line}");
            bail!("worker '{id}' closed")
        }
        WaitResult::Timeout => bail!(
            "timeout: no actionable wake line appeared in {}",
            target.status()
        ),
    }
}

enum WaitTarget {
    Run {
        status: Utf8PathBuf,
    },
    Worker {
        id: String,
        status: Utf8PathBuf,
        dir: Utf8PathBuf,
    },
}

enum WaitResult {
    Line(String),
    WorkerClosed { id: String, line: String },
    Timeout,
}

impl WaitTarget {
    fn status(&self) -> &Utf8Path {
        match self {
            WaitTarget::Run { status } | WaitTarget::Worker { status, .. } => status,
        }
    }

    fn closed_if_missing(&self) -> Option<WaitResult> {
        match self {
            WaitTarget::Run { .. } => None,
            WaitTarget::Worker { id, dir, .. } if !dir.exists() => Some(WaitResult::WorkerClosed {
                id: id.clone(),
                line: format!("closed: worker '{id}' directory removed"),
            }),
            WaitTarget::Worker { .. } => None,
        }
    }

    fn result_for_line(&self, line: String) -> WaitResult {
        match self {
            WaitTarget::Worker { id, .. } if is_closed_wake(&line) => WaitResult::WorkerClosed {
                id: id.clone(),
                line,
            },
            _ => WaitResult::Line(line),
        }
    }
}

fn wait_for_wake(
    target: &WaitTarget,
    index: Option<usize>,
    interval: Duration,
    timeout: Duration,
) -> Result<WaitResult> {
    if let Some(result) = target.closed_if_missing() {
        return Ok(result);
    }

    let mut cursor = read_lines(target.status())?.len();
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(result) = target.closed_if_missing() {
            return Ok(result);
        }

        let lines = read_lines(target.status())?;
        let start = if index.is_some() {
            0
        } else {
            cursor.min(lines.len())
        };
        if let Some(line) = select_wake(&lines[start..], index) {
            return Ok(target.result_for_line(line));
        }
        cursor = lines.len();

        if let Some(result) = target.closed_if_missing() {
            return Ok(result);
        }

        let now = Instant::now();
        if now >= deadline {
            return Ok(WaitResult::Timeout);
        }

        thread::sleep(interval.min(deadline - now));
    }
}

fn worker_status_dir(status: &Utf8Path) -> Result<Utf8PathBuf> {
    status
        .parent()
        .map(Utf8Path::to_path_buf)
        .with_context(|| format!("worker status path has no parent directory: {status}"))
}

fn select_wake(lines: &[String], index: Option<usize>) -> Option<String> {
    for line in lines {
        if !is_actionable_wake(line) {
            continue;
        }

        match index {
            Some(index) if is_closed_wake(line) || mentions_step(line, index) => {
                return Some(line.clone());
            }
            Some(_) => {}
            None => return Some(line.clone()),
        }
    }

    None
}

fn is_actionable_wake(line: &str) -> bool {
    matches!(
        wake_state(line),
        Some("done" | "failed" | "needs-decision" | "blocked" | "closed")
    )
}

fn is_closed_wake(line: &str) -> bool {
    wake_state(line) == Some("closed")
}

fn wake_state(line: &str) -> Option<&str> {
    let (state, _) = line.split_once(':')?;
    Some(state)
}

fn mentions_step(line: &str, index: usize) -> bool {
    let index = index.to_string();
    let mut previous_was_step = false;

    for token in line.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        if previous_was_step && token == index {
            return true;
        }
        previous_was_step = token == "step";
    }

    false
}

fn read_lines(status: &Utf8Path) -> Result<Vec<String>> {
    match fs::read_to_string(status) {
        Ok(body) => Ok(body.lines().map(str::to_owned).collect()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(err).with_context(|| format!("failed to read {status}")),
    }
}

fn positive_seconds_duration(seconds: f64, label: &str) -> Result<Duration> {
    if !seconds.is_finite() || seconds <= 0.0 {
        bail!("{label} must be a finite positive number");
    }
    Ok(Duration::from_secs_f64(seconds))
}

fn non_negative_seconds_duration(seconds: f64, label: &str) -> Result<Duration> {
    if !seconds.is_finite() || seconds < 0.0 {
        bail!("{label} must be a finite non-negative number");
    }
    if seconds == 0.0 {
        Ok(Duration::ZERO)
    } else {
        Ok(Duration::from_secs_f64(seconds))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_indexed_wake_before_earlier_generic_wake() {
        let lines = vec![
            "done: generic wake".to_owned(),
            "done: step 2 finished".to_owned(),
        ];

        assert_eq!(
            select_wake(&lines, Some(2)).as_deref(),
            Some("done: step 2 finished")
        );
    }

    #[test]
    fn indexed_wake_ignores_generic_and_other_step_wakes() {
        let lines = vec![
            "done: generic wake".to_owned(),
            "failed: step 1 failed".to_owned(),
        ];

        assert_eq!(select_wake(&lines, Some(2)), None);
    }

    #[test]
    fn indexed_wake_keeps_closed_terminal_backstop() {
        let lines = vec![
            "done: generic wake".to_owned(),
            "closed: auth-fix".to_owned(),
        ];

        assert_eq!(
            select_wake(&lines, Some(2)).as_deref(),
            Some("closed: auth-fix")
        );
    }

    #[test]
    fn step_matching_does_not_confuse_prefixes() {
        assert!(!mentions_step("done: step 10 finished", 1));
        assert!(mentions_step("done: step 1 finished", 1));
    }

    #[test]
    fn closed_is_actionable_and_terminal() {
        assert!(is_actionable_wake("closed: auth-fix"));
        assert!(is_closed_wake("closed: auth-fix"));

        let lines = vec![
            "working: teardown".to_owned(),
            "closed: auth-fix".to_owned(),
        ];
        assert_eq!(
            select_wake(&lines, None).as_deref(),
            Some("closed: auth-fix")
        );
    }
}
