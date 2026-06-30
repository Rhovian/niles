use std::{
    fs, thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

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

    let status = match (run, crew) {
        (Some(run), None) => store::resolve_run_dir(&run)?.join("status.log"),
        (None, Some(id)) => crew::status_log_path(&id)?,
        (Some(_), Some(_)) => bail!("use either a run id or --crew <id>, not both"),
        (None, None) => bail!("wait requires a run id or --crew <id>"),
    };

    let interval = positive_seconds_duration(interval, "wait interval")?;
    let timeout = timeout
        .map(|seconds| non_negative_seconds_duration(seconds, "wait timeout"))
        .transpose()?;

    match wait_for_wake(&status, index, interval, timeout)? {
        Some(line) => {
            println!("{line}");
            Ok(())
        }
        None => bail!("timeout: no actionable wake line appeared in {status}"),
    }
}

fn wait_for_wake(
    status: &Utf8Path,
    index: Option<usize>,
    interval: Duration,
    timeout: Option<Duration>,
) -> Result<Option<String>> {
    let mut cursor = read_lines(status)?.len();
    let deadline = timeout.map(|timeout| Instant::now() + timeout);

    loop {
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

        let lines = read_lines(status)?;
        let start = cursor.min(lines.len());
        if let Some(line) = select_wake(&lines[start..], index) {
            return Ok(Some(line));
        }
        cursor = lines.len();

        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Ok(None);
        }
    }
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
    let Some((state, _)) = line.split_once(':') else {
        return false;
    };
    matches!(state, "done" | "failed" | "needs-decision" | "blocked")
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
}
