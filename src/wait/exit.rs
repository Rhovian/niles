use std::{process::ExitCode, time::Duration};

use camino::Utf8PathBuf;

use crate::wake::{self, WakeKind};

pub(super) enum Outcome {
    Wake {
        id: String,
        line: String,
    },
    Closed {
        id: String,
        status: Utf8PathBuf,
        line: String,
    },
    WindowGone {
        id: String,
        status: Utf8PathBuf,
    },
    Timeout {
        subject: String,
        timeout: Duration,
    },
}

pub(crate) struct WaitExit {
    code: u8,
    stdout: Option<String>,
    stderr: Option<String>,
}

impl WaitExit {
    pub(super) fn from_outcome(outcome: Outcome, prefix_worker_id: bool) -> Self {
        match outcome {
            Outcome::Wake { id, line } => Self {
                code: super::EXIT_WAKE,
                stdout: Some(wake_line(&id, line, prefix_worker_id)),
                stderr: None,
            },
            Outcome::Closed { id, status, line } => Self {
                code: super::EXIT_WORKER_CLOSED,
                stdout: Some(wake_line(&id, line, prefix_worker_id)),
                stderr: Some(format!(
                    "wait: worker-closed worker={} status={} detail={}",
                    field(&id),
                    field(status.as_str()),
                    detail(&format!("worker '{id}' closed"))
                )),
            },
            Outcome::WindowGone { id, status } => Self {
                code: super::EXIT_WORKER_CLOSED,
                stdout: Some(wake_line(
                    &id,
                    wake::line(
                        WakeKind::Failed,
                        &format!("worker '{id}' exited without reporting; its window is gone"),
                    ),
                    prefix_worker_id,
                )),
                stderr: Some(format!(
                    "wait: window-gone worker={} status={} detail={}",
                    field(&id),
                    field(status.as_str()),
                    detail(&format!(
                        "worker '{id}' tmux window is gone and its status log ended without a final line"
                    ))
                )),
            },
            Outcome::Timeout { subject, timeout } => Self {
                code: super::EXIT_TIMEOUT,
                stdout: None,
                stderr: Some(format!(
                    "wait: timeout target={} timeout={}",
                    field(&subject),
                    format_duration(timeout)
                )),
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

fn wake_line(id: &str, line: String, prefix_worker_id: bool) -> String {
    if prefix_worker_id {
        return format!("{id}: {line}");
    }
    line
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
    serde_json::Value::from(value).to_string()
}
