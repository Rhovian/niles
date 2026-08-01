use anyhow::Result;
use camino::Utf8Path;
use chrono::Utc;

use crate::util::append_line;

use super::super::scanner::ack_log_path;
use super::WaiterRegistration;

pub(super) fn log_stale_waiter_reclaimed(
    status: &Utf8Path,
    waiter: &WaiterRegistration,
    reason: &str,
) -> Result<()> {
    let path = ack_log_path(status);
    let record = serde_json::json!({
        "event": "stale-waiter-reclaimed",
        "pid": waiter.pid,
        "started_at": waiter.started_at.to_rfc3339(),
        "token": &waiter.token,
        "reason": reason,
        "reclaimed_at": Utc::now().to_rfc3339(),
    });
    append_json_record(&path, record)
}

pub(super) fn log_waiter_takeover_requested(
    status: &Utf8Path,
    waiter: &WaiterRegistration,
) -> Result<()> {
    let path = ack_log_path(status);
    let record = serde_json::json!({
        "event": "waiter-takeover-requested",
        "pid": waiter.pid,
        "started_at": waiter.started_at.to_rfc3339(),
        "token": &waiter.token,
        "requested_at": Utc::now().to_rfc3339(),
        "signal": "SIGTERM",
    });
    append_json_record(&path, record)
}

pub(super) fn log_waiter_taken_over(status: &Utf8Path, waiter: &WaiterRegistration) -> Result<()> {
    let path = ack_log_path(status);
    let record = serde_json::json!({
        "event": "waiter-taken-over",
        "pid": waiter.pid,
        "started_at": waiter.started_at.to_rfc3339(),
        "token": &waiter.token,
        "taken_over_at": Utc::now().to_rfc3339(),
    });
    append_json_record(&path, record)
}

fn append_json_record(path: &Utf8Path, record: serde_json::Value) -> Result<()> {
    let line = serde_json::to_string(&record)?;
    append_line(
        path,
        &line,
        |path| format!("failed to open {path}"),
        |path| format!("failed to inspect {path}"),
        |path| format!("failed to write {path}"),
    )
}
