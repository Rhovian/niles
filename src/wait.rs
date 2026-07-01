use std::{
    fs, thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{crew, store};

pub fn wait(
    run: Option<String>,
    crew: Option<String>,
    index: Option<usize>,
    interval: f64,
    timeout: Option<f64>,
) -> Result<()> {
    if let Some(0) = index {
        bail!("step index must be >= 1");
    }

    let target = match (run, crew) {
        (Some(run), None) => WaitTarget::Run {
            status: store::resolve_run_dir(&run)?.join("status.log"),
        },
        (None, Some(id)) => {
            let status = crew::status_log_path(&id)?;
            let dir = crew_status_dir(&status)?;
            WaitTarget::Crew { id, status, dir }
        }
        (Some(_), Some(_)) => bail!("use either a run id or --crew <id>, not both"),
        (None, None) => bail!("wait requires a run id or --crew <id>"),
    };

    let interval = positive_seconds_duration(interval, "wait interval")?;
    let timeout = timeout
        .map(|seconds| non_negative_seconds_duration(seconds, "wait timeout"))
        .transpose()?;

    match wait_for_wake(&target, index, interval, timeout)? {
        WaitResult::Line(line) => {
            println!("{line}");
            Ok(())
        }
        WaitResult::CrewClosed { id, line } => {
            println!("{line}");
            bail!("crew '{id}' closed")
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
    Crew {
        id: String,
        status: Utf8PathBuf,
        dir: Utf8PathBuf,
    },
}

enum WaitResult {
    Line(String),
    CrewClosed { id: String, line: String },
    Timeout,
}

impl WaitTarget {
    fn status(&self) -> &Utf8Path {
        match self {
            WaitTarget::Run { status } | WaitTarget::Crew { status, .. } => status,
        }
    }

    fn closed_if_missing(&self) -> Option<WaitResult> {
        match self {
            WaitTarget::Run { .. } => None,
            WaitTarget::Crew { id, dir, .. } if !dir.exists() => Some(WaitResult::CrewClosed {
                id: id.clone(),
                line: format!("closed: crew '{id}' directory removed"),
            }),
            WaitTarget::Crew { .. } => None,
        }
    }

    fn result_for_line(&self, line: String) -> WaitResult {
        match self {
            WaitTarget::Crew { id, .. } if is_closed_wake(&line) => WaitResult::CrewClosed {
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
    timeout: Option<Duration>,
) -> Result<WaitResult> {
    if let Some(result) = target.closed_if_missing() {
        return Ok(result);
    }

    let mut cursor = read_lines(target.status())?.len();
    let deadline = timeout.map(|timeout| Instant::now() + timeout);

    loop {
        if let Some(result) = target.closed_if_missing() {
            return Ok(result);
        }

        let sleep_for = match deadline {
            Some(deadline) => {
                let now = Instant::now();
                if now >= deadline {
                    Duration::ZERO
                } else {
                    interval.min(deadline - now)
                }
            }
            None => interval,
        };

        if !sleep_for.is_zero() {
            thread::sleep(sleep_for);
        }

        let lines = read_lines(target.status())?;
        let start = cursor.min(lines.len());
        if let Some(line) = select_wake(&lines[start..], index) {
            return Ok(target.result_for_line(line));
        }
        cursor = lines.len();

        if let Some(result) = target.closed_if_missing() {
            return Ok(result);
        }

        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Ok(WaitResult::Timeout);
        }
    }
}

fn crew_status_dir(status: &Utf8Path) -> Result<Utf8PathBuf> {
    status
        .parent()
        .map(Utf8Path::to_path_buf)
        .with_context(|| format!("crew status path has no parent directory: {status}"))
}

fn select_wake(lines: &[String], index: Option<usize>) -> Option<String> {
    let mut first_wake = None;

    for line in lines {
        if !is_actionable_wake(line) {
            continue;
        }

        if first_wake.is_none() {
            first_wake = Some(line.clone());
        }

        if let Some(index) = index
            && mentions_step(line, index)
        {
            return Some(line.clone());
        }
    }

    first_wake
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
