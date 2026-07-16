mod guard;
mod scanner;
mod target;

use std::{
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::util::read_optional_to_string;

use guard::WaiterGuard;
use scanner::WakeScanner;
use target::{WaitTarget, WaitTargets};

const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(60 * 60);

pub fn wait(
    worker: Vec<String>,
    task: Option<String>,
    interval: f64,
    timeout: Option<f64>,
) -> Result<()> {
    let targets = WaitTargets::resolve(worker, task)?;
    let prefix_worker_id = targets.prefix_worker_id();
    let timeout_subject = targets.timeout_subject();

    let interval = positive_seconds_duration(interval, "wait interval")?;
    let timeout = match timeout
        .map(|seconds| non_negative_seconds_duration(seconds, "wait timeout"))
        .transpose()?
    {
        Some(timeout) => timeout,
        None => DEFAULT_WAIT_TIMEOUT,
    };

    let wake = wait_for_targets(targets, interval, timeout)?;
    let worker_id = wake.worker_id;
    match wake.result {
        WaitResult::Line(line) => {
            println!(
                "{}",
                format_wake_line(worker_id.as_deref(), line, prefix_worker_id)
            );
            Ok(())
        }
        WaitResult::WorkerClosed { id, line } => {
            println!(
                "{}",
                format_wake_line(worker_id.as_deref(), line, prefix_worker_id)
            );
            bail!("worker '{id}' closed")
        }
        WaitResult::Timeout => {
            bail!("timeout: no actionable wake line appeared in {timeout_subject}")
        }
    }
}

enum WaitResult {
    Line(String),
    WorkerClosed { id: String, line: String },
    Timeout,
}

struct WaitEntry {
    target: WaitTarget,
    scanner: WakeScanner,
    guard: Option<WaiterGuard>,
}

struct FiredWake {
    worker_id: Option<String>,
    result: WaitResult,
}

fn format_wake_line(worker_id: Option<&str>, line: String, prefix_worker_id: bool) -> String {
    if prefix_worker_id && let Some(id) = worker_id {
        return format!("{id}: {line}");
    }
    line
}

fn wait_for_targets(
    targets: WaitTargets,
    interval: Duration,
    timeout: Duration,
) -> Result<FiredWake> {
    match targets {
        WaitTargets::Workers(mut targets) if targets.len() == 1 => {
            let target = targets.remove(0);
            let worker_id = target.worker_id().map(str::to_owned);
            Ok(FiredWake {
                worker_id,
                result: wait_for_wake(target, interval, timeout)?,
            })
        }
        WaitTargets::Workers(targets) => wait_for_first_wake(targets, interval, timeout),
    }
}

fn wait_for_wake(target: WaitTarget, interval: Duration, timeout: Duration) -> Result<WaitResult> {
    Ok(wait_for_first_wake(vec![target], interval, timeout)?.result)
}

fn wait_for_first_wake(
    targets: Vec<WaitTarget>,
    interval: Duration,
    timeout: Duration,
) -> Result<FiredWake> {
    // Missing worker dirs report WorkerClosed before waiter registration can fail on status.waiter.
    for target in &targets {
        if let Some(result) = target.closed_if_missing() {
            return Ok(FiredWake {
                worker_id: target.worker_id().map(str::to_owned),
                result,
            });
        }
    }

    let mut entries = prepare_wait_entries(targets)?;
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(wake) = poll_wait_entries(&mut entries)? {
            return Ok(wake);
        }

        let now = Instant::now();
        if now >= deadline {
            return Ok(FiredWake {
                worker_id: None,
                result: WaitResult::Timeout,
            });
        }

        thread::sleep(interval.min(deadline - now));
    }
}

fn prepare_wait_entries(targets: Vec<WaitTarget>) -> Result<Vec<WaitEntry>> {
    let mut guards = Vec::with_capacity(targets.len());
    for target in &targets {
        guards.push(Some(WaiterGuard::register(target.status()).with_context(
            || {
                format!(
                    "failed to attach waiter guard for requested target {}",
                    target.status()
                )
            },
        )?));
    }

    let mut entries = Vec::with_capacity(targets.len());
    for (target, guard) in targets.into_iter().zip(guards) {
        let initial_lines = read_lines(target.status())?;
        let scanner = WakeScanner::new(
            &target,
            initial_lines.len(),
            guard.as_ref().map(|guard| guard.registration.clone()),
        )?;
        entries.push(WaitEntry {
            target,
            scanner,
            guard,
        });
    }
    Ok(entries)
}

fn poll_wait_entries(entries: &mut [WaitEntry]) -> Result<Option<FiredWake>> {
    for entry in entries.iter_mut() {
        if let Some(result) = entry.target.closed_if_missing() {
            return Ok(Some(FiredWake {
                worker_id: entry.target.worker_id().map(str::to_owned),
                result,
            }));
        }
        if let Some(waiter) = &entry.guard
            && let Err(err) = waiter.verify()
        {
            if let Some(result) = entry.target.closed_if_missing() {
                return Ok(Some(FiredWake {
                    worker_id: entry.target.worker_id().map(str::to_owned),
                    result,
                }));
            }
            return Err(err);
        }

        let lines = read_lines(entry.target.status())?;
        if let Some(result) = entry.scanner.select(&entry.target, &lines)? {
            return Ok(Some(FiredWake {
                worker_id: entry.target.worker_id().map(str::to_owned),
                result,
            }));
        }

        if let Some(result) = entry.target.closed_if_missing() {
            return Ok(Some(FiredWake {
                worker_id: entry.target.worker_id().map(str::to_owned),
                result,
            }));
        }
    }
    Ok(None)
}

fn read_lines(status: &Utf8Path) -> Result<Vec<String>> {
    Ok(status_lines_or_empty(read_optional_to_string(
        status,
        |status| format!("failed to read {status}"),
    )?))
}

fn status_lines_or_empty(body: Option<String>) -> Vec<String> {
    match body {
        Some(body) => body.lines().map(str::to_owned).collect(),
        None => Vec::new(),
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
mod test_support {
    use super::{WaitResult, target::WaitTarget};
    use camino::{Utf8Path, Utf8PathBuf};
    use std::{
        env, fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    pub(in crate::wait) fn assert_wait_line(result: WaitResult, expected: &str) {
        let WaitResult::Line(line) = result else {
            panic!("expected line result");
        };
        assert_eq!(line, expected);
    }

    pub(in crate::wait) fn test_worker_target(
        root: &Utf8Path,
        id: &str,
        status_body: &str,
    ) -> WaitTarget {
        let dir = root.join(id);
        fs::create_dir_all(&dir).unwrap();
        let status = dir.join("status.log");
        fs::write(&status, status_body).unwrap();
        WaitTarget::Worker {
            id: id.to_owned(),
            status,
            dir,
        }
    }

    pub(in crate::wait) fn write_waiter_registration(status: &Utf8Path, pid: u32, token: &str) {
        fs::write(
            super::guard::waiter_path(status),
            format!(
                r#"{{
  "pid": {pid},
  "started_at": "2000-01-01T00:00:00Z",
  "token": "{token}"
}}
"#
            ),
        )
        .unwrap();
    }

    pub(in crate::wait) struct TestTempDir {
        path: Utf8PathBuf,
    }

    impl TestTempDir {
        pub(in crate::wait) fn new(name: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                env::temp_dir().join(format!("niles-wait-{name}-{}-{nanos}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self {
                path: Utf8PathBuf::from_path_buf(path).unwrap(),
            }
        }

        pub(in crate::wait) fn path(&self) -> &Utf8Path {
            &self.path
        }
    }

    impl Drop for TestTempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wait::{
        guard::waiter_path,
        scanner::{ack_log_path, ack_path},
        test_support::{TestTempDir, assert_wait_line, test_worker_target},
    };
    use std::{fs, time::Duration};

    #[test]
    fn tie_break_returns_first_in_target_order() {
        let root = TestTempDir::new("tie-break");
        let first = test_worker_target(root.path(), "second", "done: second worker\n");
        let second = test_worker_target(root.path(), "first", "done: first worker\n");
        let wake = wait_for_first_wake(
            vec![first, second],
            Duration::from_millis(1),
            Duration::ZERO,
        )
        .unwrap();

        assert_eq!(wake.worker_id.as_deref(), Some("second"));
        assert_wait_line(wake.result, "done: second worker");
    }

    #[test]
    fn single_element_set_matches_single_wait() {
        let root = TestTempDir::new("single-element");
        let target = test_worker_target(root.path(), "only", "done: single\n");
        let status = target.status().to_path_buf();
        let wake = wait_for_targets(
            WaitTargets::Workers(vec![target]),
            Duration::from_millis(1),
            Duration::ZERO,
        )
        .unwrap();

        let WaitResult::Line(line) = wake.result else {
            panic!("expected line result");
        };
        assert_eq!(
            format_wake_line(wake.worker_id.as_deref(), line, false),
            "done: single"
        );
        assert_eq!(fs::read_to_string(ack_path(&status)).unwrap(), "1\n");
        assert!(!waiter_path(&status).exists());
    }

    #[test]
    fn only_firing_targets_cursor_advances() {
        let root = TestTempDir::new("cursor-isolation");
        let first = test_worker_target(root.path(), "first", "done: first\n");
        let second = test_worker_target(root.path(), "second", "done: second\n");
        let mut entries = prepare_wait_entries(vec![first, second]).unwrap();

        let wake = poll_wait_entries(&mut entries).unwrap().unwrap();

        assert_eq!(wake.worker_id.as_deref(), Some("first"));
        assert_eq!(entries[0].scanner.cursor, 1);
        assert_eq!(entries[1].scanner.cursor, 0);
    }

    #[test]
    fn ack_only_the_firing_worker() {
        let root = TestTempDir::new("ack-isolation");
        let first = test_worker_target(root.path(), "first", "working: first\n");
        let first_ack = ack_path(first.status());
        let first_ack_log = ack_log_path(first.status());
        let second = test_worker_target(root.path(), "second", "done: second\n");
        let second_ack = ack_path(second.status());
        let second_ack_log = ack_log_path(second.status());
        let third = test_worker_target(root.path(), "third", "done: third\n");
        let third_ack = ack_path(third.status());
        let third_ack_log = ack_log_path(third.status());

        let wake = wait_for_first_wake(
            vec![first, second, third],
            Duration::from_millis(1),
            Duration::ZERO,
        )
        .unwrap();

        assert_eq!(wake.worker_id.as_deref(), Some("second"));
        assert!(!first_ack.exists());
        assert!(!first_ack_log.exists());
        assert_eq!(fs::read_to_string(second_ack).unwrap(), "1\n");
        assert!(
            fs::read_to_string(second_ack_log)
                .unwrap()
                .contains("done: second")
        );
        assert!(!third_ack.exists());
        assert!(!third_ack_log.exists());
    }
}
