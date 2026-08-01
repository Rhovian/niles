use std::{process::ExitCode, time::Duration};

use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};

const UNKNOWN_WORKER: &str = "unknown";

pub const EXIT_WAKE: u8 = 0;
pub const EXIT_WORKER_CLOSED: u8 = 10;
#[allow(dead_code)]
pub const EXIT_SILENT_STOP: u8 = 20;
pub const EXIT_WAITER_CONFLICT: u8 = 21;
pub const EXIT_TIMEOUT: u8 = 22;
pub const EXIT_INDETERMINATE: u8 = 23;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::wait) enum HeartbeatState {
    Fresh,
    Missing,
    Stale,
}

impl HeartbeatState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Missing => "missing",
            Self::Stale => "stale",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::wait) struct WaiterConflict {
    pub(in crate::wait) worker_id: Option<String>,
    pub(in crate::wait) status: Utf8PathBuf,
    pub(in crate::wait) waiter: Utf8PathBuf,
    pub(in crate::wait) holder_pid: u32,
    pub(in crate::wait) holder_since: DateTime<Utc>,
    pub(in crate::wait) heartbeat: HeartbeatState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::wait) struct WaitIndeterminate {
    pub(in crate::wait) worker_id: Option<String>,
    pub(in crate::wait) status: Option<Utf8PathBuf>,
    pub(in crate::wait) waiter: Option<Utf8PathBuf>,
    pub(in crate::wait) detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::wait) struct WaiterTakeoverFailed {
    pub(in crate::wait) worker_id: Option<String>,
    pub(in crate::wait) status: Utf8PathBuf,
    pub(in crate::wait) waiter: Utf8PathBuf,
    pub(in crate::wait) holder_pid: u32,
    pub(in crate::wait) holder_since: DateTime<Utc>,
    pub(in crate::wait) detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::wait) struct WaitTimeout {
    pub(in crate::wait) target: String,
    pub(in crate::wait) timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::wait) enum WaitOutcome {
    Wake {
        worker_id: Option<String>,
        line: String,
    },
    WorkerClosed {
        worker_id: Option<String>,
        id: String,
        status: Utf8PathBuf,
        line: String,
    },
    WaiterConflict(WaiterConflict),
    Timeout(WaitTimeout),
    Indeterminate(WaitIndeterminate),
    WaiterTakeoverFailed(WaiterTakeoverFailed),
}

impl WaitOutcome {
    #[cfg(test)]
    pub(in crate::wait) fn worker_id(&self) -> Option<&str> {
        match self {
            Self::Wake { worker_id, .. }
            | Self::WorkerClosed { worker_id, .. }
            | Self::WaiterConflict(WaiterConflict { worker_id, .. })
            | Self::Indeterminate(WaitIndeterminate { worker_id, .. })
            | Self::WaiterTakeoverFailed(WaiterTakeoverFailed { worker_id, .. }) => {
                worker_id.as_deref()
            }
            Self::Timeout(_) => None,
        }
    }
}

pub(crate) struct WaitExit {
    code: u8,
    stdout: Option<String>,
    stderr: Option<String>,
}

impl WaitExit {
    pub(in crate::wait) fn from_outcome(outcome: WaitOutcome, prefix_worker_id: bool) -> Self {
        match outcome {
            WaitOutcome::Wake { worker_id, line } => Self {
                code: EXIT_WAKE,
                stdout: Some(format_wake_line(
                    worker_id.as_deref(),
                    line,
                    prefix_worker_id,
                )),
                stderr: None,
            },
            WaitOutcome::WorkerClosed {
                worker_id,
                id,
                status,
                line,
            } => Self {
                code: EXIT_WORKER_CLOSED,
                stdout: Some(format_wake_line(
                    worker_id.as_deref(),
                    line,
                    prefix_worker_id,
                )),
                stderr: Some(format!(
                    "wait: worker-closed worker={} status={} detail={}",
                    field(worker_label(worker_id.as_deref(), &id)),
                    field(status.as_str()),
                    detail(&format!("worker '{id}' closed"))
                )),
            },
            WaitOutcome::WaiterConflict(conflict) => Self {
                code: EXIT_WAITER_CONFLICT,
                stdout: None,
                stderr: Some(format_waiter_conflict(&conflict)),
            },
            WaitOutcome::Timeout(timeout) => Self {
                code: EXIT_TIMEOUT,
                stdout: None,
                stderr: Some(format!(
                    "wait: timeout target={} timeout={} idle_state=not-implemented",
                    field(&timeout.target),
                    format_duration(timeout.timeout)
                )),
            },
            WaitOutcome::Indeterminate(indeterminate) => Self {
                code: EXIT_INDETERMINATE,
                stdout: None,
                stderr: Some(format_indeterminate(&indeterminate)),
            },
            WaitOutcome::WaiterTakeoverFailed(failed) => Self {
                code: EXIT_INDETERMINATE,
                stdout: None,
                stderr: Some(format_takeover_failed(&failed)),
            },
        }
    }

    pub(crate) fn emit(self) -> ExitCode {
        if let Some(stdout) = self.stdout {
            println!("{stdout}");
        }
        if let Some(stderr) = self.stderr {
            eprintln!("{stderr}");
        }
        ExitCode::from(self.code)
    }
}

pub(in crate::wait) fn format_wake_line(
    worker_id: Option<&str>,
    line: String,
    prefix_worker_id: bool,
) -> String {
    if prefix_worker_id && let Some(id) = worker_id {
        return format!("{id}: {line}");
    }
    line
}

fn format_waiter_conflict(conflict: &WaiterConflict) -> String {
    let worker = worker_label(conflict.worker_id.as_deref(), UNKNOWN_WORKER);
    format!(
        "wait: waiter-conflict worker={} status={} waiter={} holder_pid={} holder_since={} heartbeat={} hint={}",
        field(worker),
        field(conflict.status.as_str()),
        field(conflict.waiter.as_str()),
        conflict.holder_pid,
        field(&conflict.holder_since.to_rfc3339()),
        conflict.heartbeat.as_str(),
        quoted(&format!("niles wait --worker {worker} --takeover"))
    )
}

fn format_indeterminate(indeterminate: &WaitIndeterminate) -> String {
    let mut fields = vec!["wait: indeterminate".to_owned()];
    if let Some(worker_id) = indeterminate.worker_id.as_deref() {
        fields.push(format!("worker={}", field(worker_id)));
    }
    if let Some(status) = &indeterminate.status {
        fields.push(format!("status={}", field(status.as_str())));
    }
    if let Some(waiter) = &indeterminate.waiter {
        fields.push(format!("waiter={}", field(waiter.as_str())));
    }
    fields.push(format!("detail={}", detail(&indeterminate.detail)));
    fields.join(" ")
}

fn format_takeover_failed(failed: &WaiterTakeoverFailed) -> String {
    format!(
        "wait: waiter-takeover-failed worker={} status={} waiter={} holder_pid={} holder_since={} detail={}",
        field(worker_label(failed.worker_id.as_deref(), UNKNOWN_WORKER)),
        field(failed.status.as_str()),
        field(failed.waiter.as_str()),
        failed.holder_pid,
        field(&failed.holder_since.to_rfc3339()),
        detail(&failed.detail)
    )
}

fn format_duration(duration: Duration) -> String {
    let nanos = duration.as_nanos();
    if nanos == 0 {
        return "0s".to_owned();
    }
    if nanos.is_multiple_of(1_000_000_000) {
        return format!("{}s", duration.as_secs());
    }
    if nanos.is_multiple_of(1_000_000) {
        return format!("{}ms", nanos / 1_000_000);
    }
    format!("{:.3}s", duration.as_secs_f64())
}

fn field(value: &str) -> String {
    if value.is_empty()
        || value
            .chars()
            .any(|ch| ch.is_whitespace() || ch == '\'' || ch == '"')
    {
        quoted(value)
    } else {
        value.to_owned()
    }
}

fn quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn detail(value: &str) -> String {
    match serde_json::to_string(value) {
        Ok(json) => json,
        Err(_) => quoted(value),
    }
}

fn worker_label<'a>(worker_id: Option<&'a str>, fallback: &'a str) -> &'a str {
    match worker_id {
        Some(worker_id) => worker_id,
        None => fallback,
    }
}
