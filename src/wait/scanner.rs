use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    util::{append_line, read_optional_to_string},
    wake::is_actionable_wake,
};

use super::{WaitResult, guard::WaiterRegistration, target::WaitTarget};

pub(in crate::wait) struct WakeScanner {
    pub(in crate::wait) cursor: usize,
    waiter: Option<WaiterRegistration>,
}

impl WakeScanner {
    pub(in crate::wait) fn new(
        target: &WaitTarget,
        initial_line_count: usize,
        waiter: Option<WaiterRegistration>,
    ) -> Result<Self> {
        let cursor = read_ack_cursor(target.status(), initial_line_count)?;

        Ok(Self { cursor, waiter })
    }

    pub(in crate::wait) fn select(
        &mut self,
        target: &WaitTarget,
        lines: &[String],
    ) -> Result<Option<WaitResult>> {
        let start = self.cursor.min(lines.len());
        if let Some((offset, line)) = select_wake_with_offset(&lines[start..]) {
            self.acknowledge(target.status(), start + offset + 1, &line)?;
            return Ok(Some(target.result_for_line(line)));
        }

        self.cursor = lines.len();
        Ok(None)
    }

    fn acknowledge(&mut self, status: &Utf8Path, cursor: usize, line: &str) -> Result<()> {
        if let Some(waiter) = &self.waiter {
            log_ack_consumption(status, cursor, line, waiter)?;
        }
        write_ack_cursor(status, cursor)?;
        self.cursor = cursor;
        Ok(())
    }
}

#[cfg(test)]
fn select_wake(lines: &[String]) -> Option<String> {
    select_wake_with_offset(lines).map(|(_, line)| line)
}

fn select_wake_with_offset(lines: &[String]) -> Option<(usize, String)> {
    for (offset, line) in lines.iter().enumerate() {
        if is_actionable_wake(line) {
            return Some((offset, line.clone()));
        }
    }

    None
}

pub(in crate::wait) fn ack_path(status: &Utf8Path) -> Utf8PathBuf {
    status.with_extension("ack")
}

pub(in crate::wait) fn ack_log_path(status: &Utf8Path) -> Utf8PathBuf {
    status.with_extension("ack.log")
}

fn read_ack_cursor(status: &Utf8Path, line_count: usize) -> Result<usize> {
    let path = ack_path(status);
    let Some(body) = read_optional_to_string(&path, |path| format!("failed to read {path}"))?
    else {
        return Ok(0);
    };

    let cursor = body
        .trim()
        .parse::<usize>()
        .with_context(|| format!("invalid ack cursor in {path}"))?;
    Ok(cursor.min(line_count))
}

fn write_ack_cursor(status: &Utf8Path, cursor: usize) -> Result<()> {
    let path = ack_path(status);
    fs::write(&path, format!("{cursor}\n")).with_context(|| format!("failed to write {path}"))
}

fn log_ack_consumption(
    status: &Utf8Path,
    cursor: usize,
    line: &str,
    waiter: &WaiterRegistration,
) -> Result<()> {
    let path = ack_log_path(status);
    let record = serde_json::json!({
        "event": "wake-consumed",
        "cursor": cursor,
        "pid": waiter.pid,
        "started_at": waiter.started_at.to_rfc3339(),
        "token": &waiter.token,
        "line": line,
    });
    let line = serde_json::to_string(&record)?;
    append_line(
        &path,
        &line,
        |path| format!("failed to open {path}"),
        |path| format!("failed to inspect {path}"),
        |path| format!("failed to write {path}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_first_actionable_wake() {
        let lines = vec![
            "working: still running".to_owned(),
            "done: ready".to_owned(),
            "failed: later".to_owned(),
        ];

        assert_eq!(select_wake(&lines).as_deref(), Some("done: ready"));
    }
}
