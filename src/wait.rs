use std::{
    fs,
    io::ErrorKind,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};

use crate::{
    state::{RunState, StepStatus},
    store,
    util::{read_optional_json, read_optional_to_string},
    wake::{
        WakeKind, is_actionable_wake, is_closed_wake, is_untagged_actionable_wake, mentions_step,
        status_log_path,
    },
    worker,
};

#[cfg(test)]
use crate::wake::mentions_any_step;

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
        (Some(run), None) => {
            let run_dir = store::resolve_run_dir(&run)?;
            WaitTarget::Run {
                status: status_log_path(&run_dir),
                run_dir,
            }
        }
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
        run_dir: Utf8PathBuf,
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
            WaitTarget::Run { status, .. } | WaitTarget::Worker { status, .. } => status,
        }
    }

    fn closed_wake_match(&self) -> ClosedWakeMatch {
        match self {
            WaitTarget::Run { .. } => ClosedWakeMatch::StepMentionOnly,
            WaitTarget::Worker { .. } => ClosedWakeMatch::AnyClosed,
        }
    }

    fn attributed_step(&self, index: usize) -> Result<Option<AttributedStep>> {
        let WaitTarget::Run { run_dir, .. } = self else {
            return Ok(None);
        };
        let Some(state) = read_optional_run_state(run_dir)? else {
            return Ok(None);
        };
        let Some(attributed) = uniquely_attributed_step(&state) else {
            return Ok(None);
        };
        Ok((attributed.index == index).then_some(attributed))
    }

    fn closed_if_missing(&self) -> Option<WaitResult> {
        match self {
            WaitTarget::Run { .. } => None,
            WaitTarget::Worker { id, dir, .. } if !dir.exists() => Some(WaitResult::WorkerClosed {
                id: id.clone(),
                line: crate::wake::line(
                    WakeKind::Closed,
                    &format!("worker '{id}' directory removed"),
                ),
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

    let initial_lines = read_lines(target.status())?;
    let mut scanner = WakeScanner::new(target, index, initial_lines.len())?;
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(result) = target.closed_if_missing() {
            return Ok(result);
        }

        let lines = read_lines(target.status())?;
        if let Some(result) = scanner.select(&lines)? {
            return Ok(result);
        }

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

struct WakeScanner<'a> {
    target: &'a WaitTarget,
    index: Option<usize>,
    cursor: usize,
}

impl<'a> WakeScanner<'a> {
    fn new(
        target: &'a WaitTarget,
        index: Option<usize>,
        initial_line_count: usize,
    ) -> Result<Self> {
        let cursor = match index {
            // Indexed waits intentionally scan the whole log for tagged wake
            // lines; the cursor only scopes the untagged attribution fallback.
            Some(_) => initial_line_count,
            // Unindexed run and worker waits consume wake lines explicitly via a
            // status.ack cursor next to the status log. That keeps pre-attach
            // wake lines visible until a waiter returns them.
            None => read_ack_cursor(target.status(), initial_line_count)?,
        };

        Ok(Self {
            target,
            index,
            cursor,
        })
    }

    fn select(&mut self, lines: &[String]) -> Result<Option<WaitResult>> {
        let start = self.direct_scan_start(lines.len());
        if let Some((offset, line)) =
            select_wake_with_offset(&lines[start..], self.index, self.target.closed_wake_match())
        {
            if self.index.is_none() {
                self.acknowledge(start + offset + 1)?;
            }
            return Ok(Some(self.target.result_for_line(line)));
        }

        if let Some(index) = self.index
            && let Some(line) =
                select_state_attributed_wake(self.target, lines, self.cursor, index)?
        {
            return Ok(Some(self.target.result_for_line(line)));
        }

        self.cursor = lines.len();
        Ok(None)
    }

    fn direct_scan_start(&self, line_count: usize) -> usize {
        if self.index.is_some() {
            0
        } else {
            self.cursor.min(line_count)
        }
    }

    fn acknowledge(&mut self, cursor: usize) -> Result<()> {
        write_ack_cursor(self.target.status(), cursor)?;
        self.cursor = cursor;
        Ok(())
    }
}

fn worker_status_dir(status: &Utf8Path) -> Result<Utf8PathBuf> {
    status
        .parent()
        .map(Utf8Path::to_path_buf)
        .with_context(|| format!("worker status path has no parent directory: {status}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClosedWakeMatch {
    AnyClosed,
    StepMentionOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AttributedStep {
    index: usize,
    started_at: DateTime<Utc>,
}

#[cfg(test)]
fn select_wake(
    lines: &[String],
    index: Option<usize>,
    closed_match: ClosedWakeMatch,
) -> Option<String> {
    select_wake_with_offset(lines, index, closed_match).map(|(_, line)| line)
}

fn select_wake_with_offset(
    lines: &[String],
    index: Option<usize>,
    closed_match: ClosedWakeMatch,
) -> Option<(usize, String)> {
    for (offset, line) in lines.iter().enumerate() {
        if !is_actionable_wake(line) {
            continue;
        }

        match index {
            Some(index) if matches_indexed_wake(line, index, closed_match) => {
                return Some((offset, line.clone()));
            }
            Some(_) => {}
            None => return Some((offset, line.clone())),
        }
    }

    None
}

fn select_state_attributed_wake(
    target: &WaitTarget,
    lines: &[String],
    cursor: usize,
    index: usize,
) -> Result<Option<String>> {
    let Some(attributed) = target.attributed_step(index)? else {
        return Ok(None);
    };
    if !status_may_contain_post_launch_lines(target.status(), attributed.started_at)? {
        return Ok(None);
    }

    let start = cursor.min(lines.len());
    Ok(select_untagged_wake(&lines[start..], WakeScanOrder::First)
        .or_else(|| select_untagged_wake(lines, WakeScanOrder::Last)))
}

#[derive(Clone, Copy)]
enum WakeScanOrder {
    First,
    Last,
}

fn select_untagged_wake(lines: &[String], order: WakeScanOrder) -> Option<String> {
    match order {
        WakeScanOrder::First => lines.iter().find(|line| is_untagged_actionable_wake(line)),
        WakeScanOrder::Last => lines
            .iter()
            .rev()
            .find(|line| is_untagged_actionable_wake(line)),
    }
    .cloned()
}

fn matches_indexed_wake(line: &str, index: usize, closed_match: ClosedWakeMatch) -> bool {
    mentions_step(line, index)
        || (closed_match == ClosedWakeMatch::AnyClosed && is_closed_wake(line))
}

fn read_optional_run_state(run_dir: &Utf8Path) -> Result<Option<RunState>> {
    let path = store::state_path(run_dir);
    read_optional_json(
        &path,
        |path| format!("failed to read {path}"),
        |path| format!("failed to parse {path}"),
    )
}

fn uniquely_attributed_step(state: &RunState) -> Option<AttributedStep> {
    let mut running = state
        .steps
        .iter()
        .filter(|step| matches!(step.status, StepStatus::Running));
    if let Some(step) = running.next() {
        if running.next().is_some() {
            return None;
        }
        return step.started_at.map(|started_at| AttributedStep {
            index: step.index,
            started_at,
        });
    }

    let mut launched = state
        .steps
        .iter()
        .filter_map(|step| step.started_at.map(|started_at| (step.index, started_at)))
        .collect::<Vec<_>>();
    launched.sort_by_key(|(_, started_at)| *started_at);
    let (index, started_at) = launched.pop()?;
    if launched
        .last()
        .is_some_and(|(_, prior)| *prior == started_at)
    {
        return None;
    }

    Some(AttributedStep { index, started_at })
}

fn status_may_contain_post_launch_lines(
    status: &Utf8Path,
    launched_at: DateTime<Utc>,
) -> Result<bool> {
    let metadata = match fs::metadata(status) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("failed to stat {status}")),
    };
    let modified = metadata
        .modified()
        .with_context(|| format!("failed to read modification time for {status}"))?;
    Ok(DateTime::<Utc>::from(modified) >= launched_at)
}

fn read_lines(status: &Utf8Path) -> Result<Vec<String>> {
    Ok(
        read_optional_to_string(status, |status| format!("failed to read {status}"))?
            .map(|body| body.lines().map(str::to_owned).collect())
            .unwrap_or_default(),
    )
}

fn ack_path(status: &Utf8Path) -> Utf8PathBuf {
    status.with_extension("ack")
}

fn read_ack_cursor(status: &Utf8Path, line_count: usize) -> Result<usize> {
    let path = ack_path(status);
    let Some(body) = read_optional_to_string(&path, |path| format!("failed to read {path}"))?
    else {
        return Ok(0);
    };

    Ok(body.trim().parse::<usize>().unwrap_or(0).min(line_count))
}

fn write_ack_cursor(status: &Utf8Path, cursor: usize) -> Result<()> {
    let path = ack_path(status);
    fs::write(&path, format!("{cursor}\n")).with_context(|| format!("failed to write {path}"))
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
    use crate::state::{RunStatus, StepKind, StepRecord};

    #[test]
    fn selects_indexed_wake_before_earlier_generic_wake() {
        let lines = vec![
            "done: generic wake".to_owned(),
            "done: step 2 finished".to_owned(),
        ];

        assert_eq!(
            select_wake(&lines, Some(2), ClosedWakeMatch::AnyClosed).as_deref(),
            Some("done: step 2 finished")
        );
    }

    #[test]
    fn indexed_wake_ignores_generic_and_other_step_wakes() {
        let lines = vec![
            "done: generic wake".to_owned(),
            "failed: step 1 failed".to_owned(),
        ];

        assert_eq!(
            select_wake(&lines, Some(2), ClosedWakeMatch::AnyClosed),
            None
        );
    }

    #[test]
    fn indexed_wake_keeps_closed_terminal_backstop() {
        let lines = vec![
            "done: generic wake".to_owned(),
            "closed: auth-fix".to_owned(),
        ];

        assert_eq!(
            select_wake(&lines, Some(2), ClosedWakeMatch::AnyClosed).as_deref(),
            Some("closed: auth-fix")
        );
    }

    #[test]
    fn indexed_run_wake_requires_closed_line_to_mention_step() {
        let lines = vec!["closed: step 1".to_owned()];

        assert_eq!(
            select_wake(&lines, Some(2), ClosedWakeMatch::StepMentionOnly),
            None
        );
        assert_eq!(
            select_wake(&lines, Some(1), ClosedWakeMatch::StepMentionOnly).as_deref(),
            Some("closed: step 1")
        );
    }

    #[test]
    fn step_matching_does_not_confuse_prefixes() {
        assert!(!mentions_step("done: step 10 finished", 1));
        assert!(mentions_step("done: step 1 finished", 1));
        assert!(mentions_any_step("done: step 10 finished"));
        assert!(!mentions_any_step("done: CONSENSUS - finished"));
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
            select_wake(&lines, None, ClosedWakeMatch::AnyClosed).as_deref(),
            Some("closed: auth-fix")
        );
    }

    #[test]
    fn attributed_wake_selects_new_untagged_lines_before_existing_lines() {
        let lines = vec![
            "done: old generic".to_owned(),
            "done: CONSENSUS - current".to_owned(),
        ];

        assert_eq!(
            select_untagged_wake(&lines[1..], WakeScanOrder::First).as_deref(),
            Some("done: CONSENSUS - current")
        );
        assert_eq!(
            select_untagged_wake(&lines, WakeScanOrder::Last).as_deref(),
            Some("done: CONSENSUS - current")
        );
    }

    #[test]
    fn attributed_wake_ignores_other_step_tagged_lines() {
        let lines = vec!["done: step 1 finished".to_owned()];

        assert_eq!(select_untagged_wake(&lines, WakeScanOrder::First), None);
    }

    #[test]
    fn state_attribution_uses_unique_running_step() {
        let started = "2026-07-02T00:00:00Z".parse().unwrap();
        let state = test_state(vec![
            test_step(1, StepStatus::Completed, Some(started)),
            test_step(2, StepStatus::Running, Some(started)),
        ]);

        assert_eq!(
            uniquely_attributed_step(&state),
            Some(AttributedStep {
                index: 2,
                started_at: started
            })
        );
    }

    #[test]
    fn state_attribution_rejects_multiple_running_steps() {
        let started = "2026-07-02T00:00:00Z".parse().unwrap();
        let state = test_state(vec![
            test_step(1, StepStatus::Running, Some(started)),
            test_step(2, StepStatus::Running, Some(started)),
        ]);

        assert_eq!(uniquely_attributed_step(&state), None);
    }

    #[test]
    fn state_attribution_uses_most_recently_launched_step_when_none_running() {
        let earlier = "2026-07-02T00:00:00Z".parse().unwrap();
        let later = "2026-07-02T00:01:00Z".parse().unwrap();
        let state = test_state(vec![
            test_step(1, StepStatus::Completed, Some(earlier)),
            test_step(2, StepStatus::Completed, Some(later)),
        ]);

        assert_eq!(
            uniquely_attributed_step(&state),
            Some(AttributedStep {
                index: 2,
                started_at: later
            })
        );
    }

    fn test_state(steps: Vec<StepRecord>) -> RunState {
        let now = "2026-07-02T00:00:00Z".parse().unwrap();
        RunState {
            id: "test-run".to_owned(),
            goal: "test".to_owned(),
            workspace: None,
            config_root: None,
            task_file: None,
            created_at: now,
            updated_at: now,
            status: RunStatus::Running,
            steps,
        }
    }

    fn test_step(
        index: usize,
        status: StepStatus,
        started_at: Option<DateTime<Utc>>,
    ) -> StepRecord {
        StepRecord {
            index,
            role: None,
            kind: StepKind::Agent,
            label: format!("step-{index}"),
            status,
            started_at,
            finished_at: None,
            exit_code: None,
            stdout: None,
            stderr: None,
            diff: None,
            context: None,
            window: None,
        }
    }
}
