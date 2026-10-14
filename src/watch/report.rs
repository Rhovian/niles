use std::{fs, io::Write, os::unix::fs::OpenOptionsExt};

use anyhow::{Context, Result};
use chrono::TimeDelta;

use super::Checkin;
use crate::{telemetry::Blocker, wake, worker::WorkerSnapshot};

pub(super) fn report(
    worker: &mut WorkerSnapshot,
    planned: Checkin,
    blocker: &Blocker,
) -> Result<bool> {
    if let Blocker::Refused(failure) = blocker
        && failure.at < planned.deadline - TimeDelta::seconds(planned.delay as i64)
    {
        return Ok(false);
    }
    let tier = worker
        .provider_tier()
        .context("terminal failure worker has no provider")?;
    let tier: String = tier
        .chars()
        .filter(|ch| !ch.is_control())
        .take(256)
        .collect();
    // Transcript text becomes a single bounded line; controls cannot create
    // another status or alter the terminal when workers prints the stored line.
    let body = match blocker {
        Blocker::Refused(failure) => {
            let message: String = failure
                .message
                .lines()
                .next()
                .into_iter()
                .flat_map(str::chars)
                .filter(|ch| !ch.is_control())
                .take(1024)
                .collect();
            format!("provider refused ({tier}): {message}")
        }
        Blocker::NoRecord => {
            format!(
                "session state unknown ({tier}): session activity unavailable after one check-in interval"
            )
        }
    };
    let line = format!("blocked: {body}\n");
    let _lock = Checkin::lock(&worker.worker_dir)?;
    if Checkin::read(&worker.worker_dir)? != Some(planned) {
        return Ok(false);
    }
    let mut status = fs::OpenOptions::new()
        .append(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(wake::status_log_path(&worker.worker_dir))?;
    if status.metadata()?.len() != worker.log_len {
        return Ok(false);
    }
    // Worker echo does not take this lock: the length check is optimistic.
    // https://doc.rust-lang.org/1.89.0/std/io/trait.Write.html#method.write_all
    status.write_all(line.as_bytes())?;
    worker.refresh_log()?;
    Ok(true)
}
