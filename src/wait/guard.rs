use std::{
    fs,
    io::{ErrorKind, Write},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::util::timestamp_id;

use super::outcome::{
    HeartbeatState, WaitIndeterminate, WaitOutcome, WaiterConflict, WaiterTakeoverFailed,
};

mod file;
mod log;
mod process;

use file::{
    RegistrationRead, RegistrationTokenCheck, read_registration, registration_body,
    remove_waiter_file, replace_registration_if_token_matches, verify_registration_token,
};
use log::{log_stale_waiter_reclaimed, log_waiter_taken_over, log_waiter_takeover_requested};
use process::SystemWaiterProcess;
pub(in crate::wait) use process::WaiterProcess;

const MIN_STALE_HEARTBEAT_MS: u64 = 15_000;
const DEFAULT_TAKEOVER_GRACE: Duration = Duration::from_millis(500);
const TAKEOVER_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(in crate::wait) struct WaiterRegistration {
    pub(in crate::wait) pid: u32,
    pub(in crate::wait) started_at: DateTime<Utc>,
    pub(in crate::wait) token: String,
    #[serde(default)]
    pub(in crate::wait) heartbeat_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub(in crate::wait) heartbeat_interval_ms: Option<u64>,
}

pub(in crate::wait) struct WaiterOptions {
    takeover: bool,
    heartbeat_interval_ms: u64,
    takeover_grace: Duration,
}

impl WaiterOptions {
    pub(in crate::wait) fn new(takeover: bool, poll_interval: Duration) -> Self {
        Self {
            takeover,
            heartbeat_interval_ms: duration_millis(poll_interval),
            takeover_grace: DEFAULT_TAKEOVER_GRACE,
        }
    }

    #[cfg(test)]
    pub(in crate::wait) fn with_takeover_grace(mut self, takeover_grace: Duration) -> Self {
        self.takeover_grace = takeover_grace;
        self
    }
}

pub(in crate::wait) enum WaiterAttach {
    Attached(WaiterGuard),
    Outcome(WaitOutcome),
}

pub(in crate::wait) enum GuardCheck {
    Verified,
    Lost { detail: String },
    Indeterminate { detail: String },
}

pub(in crate::wait) struct WaiterGuard {
    path: Utf8PathBuf,
    pub(in crate::wait) registration: WaiterRegistration,
    heartbeat_interval_ms: u64,
}

impl WaiterGuard {
    pub(in crate::wait) fn register(
        status: &Utf8Path,
        worker_id: Option<&str>,
        options: &WaiterOptions,
    ) -> Result<WaiterAttach> {
        let process = SystemWaiterProcess;
        Self::register_with_process(status, worker_id, options, &process)
    }

    pub(in crate::wait) fn register_with_process(
        status: &Utf8Path,
        worker_id: Option<&str>,
        options: &WaiterOptions,
        process: &impl WaiterProcess,
    ) -> Result<WaiterAttach> {
        let path = waiter_path(status);
        let started_at = Utc::now();
        let pid = std::process::id();
        let registration = WaiterRegistration {
            pid,
            started_at,
            token: format!("{pid}-{}", timestamp_id(&started_at)),
            heartbeat_at: Some(started_at),
            heartbeat_interval_ms: Some(options.heartbeat_interval_ms),
        };
        let body = registration_body(&registration)?;

        for _ in 0..=1 {
            match Self::try_create(
                &path,
                registration.clone(),
                &body,
                options.heartbeat_interval_ms,
            )? {
                Some(guard) => return Ok(WaiterAttach::Attached(guard)),
                None => {
                    match resolve_existing_waiter(status, worker_id, &path, options, process)? {
                        ResolveExistingWaiter::Retry => {}
                        ResolveExistingWaiter::Outcome(outcome) => {
                            return Ok(WaiterAttach::Outcome(outcome));
                        }
                    }
                }
            }
        }

        Ok(WaiterAttach::Outcome(WaitOutcome::Indeterminate(
            WaitIndeterminate {
                worker_id: worker_id.map(str::to_owned),
                status: Some(status.to_path_buf()),
                waiter: Some(path),
                detail: "waiter registration changed while attaching; retry wait".to_owned(),
            },
        )))
    }

    fn try_create(
        path: &Utf8Path,
        registration: WaiterRegistration,
        body: &str,
        heartbeat_interval_ms: u64,
    ) -> Result<Option<Self>> {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => {
                if let Err(err) = file
                    .write_all(body.as_bytes())
                    .with_context(|| format!("failed to write {path}"))
                {
                    let _ = fs::remove_file(path);
                    return Err(err);
                }
                Ok(Some(Self {
                    path: path.to_path_buf(),
                    registration,
                    heartbeat_interval_ms,
                }))
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => Ok(None),
            Err(err) => Err(err).with_context(|| format!("failed to create {path}")),
        }
    }

    pub(in crate::wait) fn heartbeat(&mut self) -> Result<GuardCheck> {
        let mut registration = match verify_registration_token(&self.path, &self.registration.token)
        {
            RegistrationTokenCheck::Matches(registration) => registration,
            RegistrationTokenCheck::Missing => {
                return Ok(GuardCheck::Lost {
                    detail: format!(
                        "waiter registration was removed/replaced while waiting: {}",
                        self.path
                    ),
                });
            }
            RegistrationTokenCheck::Changed => {
                return Ok(GuardCheck::Lost {
                    detail: format!(
                        "waiter registration was removed/replaced while waiting: {}",
                        self.path
                    ),
                });
            }
            RegistrationTokenCheck::Indeterminate { detail } => {
                return Ok(GuardCheck::Indeterminate { detail });
            }
        };

        registration.heartbeat_at = Some(Utc::now());
        registration.heartbeat_interval_ms = Some(self.heartbeat_interval_ms);

        match replace_registration_if_token_matches(
            &self.path,
            &registration,
            &self.registration.token,
        )? {
            GuardCheck::Verified => {
                self.registration = registration;
                Ok(GuardCheck::Verified)
            }
            GuardCheck::Lost { detail } => Ok(GuardCheck::Lost { detail }),
            GuardCheck::Indeterminate { detail } => Ok(GuardCheck::Indeterminate { detail }),
        }
    }

    pub(in crate::wait) fn path(&self) -> &Utf8Path {
        &self.path
    }
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        let RegistrationRead::Present(registration) = read_registration(&self.path) else {
            return;
        };

        if registration.token == self.registration.token {
            let _ = fs::remove_file(&self.path);
        }
    }
}

enum ResolveExistingWaiter {
    Retry,
    Outcome(WaitOutcome),
}

fn resolve_existing_waiter(
    status: &Utf8Path,
    worker_id: Option<&str>,
    path: &Utf8Path,
    options: &WaiterOptions,
    process: &impl WaiterProcess,
) -> Result<ResolveExistingWaiter> {
    let waiter = match read_registration(path) {
        RegistrationRead::Present(waiter) => waiter,
        RegistrationRead::Missing => return Ok(ResolveExistingWaiter::Retry),
        RegistrationRead::Indeterminate { detail } => {
            return Ok(ResolveExistingWaiter::Outcome(indeterminate(
                worker_id,
                Some(status),
                Some(path),
                detail,
            )));
        }
    };

    match process.is_running(waiter.pid) {
        Ok(false) => reclaim_waiter(status, path, &waiter, "pid-dead"),
        Ok(true) => resolve_live_waiter(status, worker_id, path, waiter, options, process),
        Err(err) => Ok(ResolveExistingWaiter::Outcome(indeterminate(
            worker_id,
            Some(status),
            Some(path),
            format!(
                "failed to determine whether waiter pid {} is running: {err:#}",
                waiter.pid
            ),
        ))),
    }
}

fn resolve_live_waiter(
    status: &Utf8Path,
    worker_id: Option<&str>,
    path: &Utf8Path,
    waiter: WaiterRegistration,
    options: &WaiterOptions,
    process: &impl WaiterProcess,
) -> Result<ResolveExistingWaiter> {
    let heartbeat = heartbeat_state(&waiter, Utc::now());
    if heartbeat == HeartbeatState::Stale {
        return reclaim_waiter(status, path, &waiter, "heartbeat-stale");
    }

    if options.takeover {
        return takeover_waiter(status, worker_id, path, waiter, options, process);
    }

    Ok(ResolveExistingWaiter::Outcome(WaitOutcome::WaiterConflict(
        WaiterConflict {
            worker_id: worker_id.map(str::to_owned),
            status: status.to_path_buf(),
            waiter: path.to_path_buf(),
            holder_pid: waiter.pid,
            holder_since: waiter.started_at,
            heartbeat,
        },
    )))
}

fn reclaim_waiter(
    status: &Utf8Path,
    path: &Utf8Path,
    waiter: &WaiterRegistration,
    reason: &str,
) -> Result<ResolveExistingWaiter> {
    log_stale_waiter_reclaimed(status, waiter, reason)?;
    remove_waiter_file(path)?;
    Ok(ResolveExistingWaiter::Retry)
}

fn takeover_waiter(
    status: &Utf8Path,
    worker_id: Option<&str>,
    path: &Utf8Path,
    waiter: WaiterRegistration,
    options: &WaiterOptions,
    process: &impl WaiterProcess,
) -> Result<ResolveExistingWaiter> {
    match verify_registration_token(path, &waiter.token) {
        RegistrationTokenCheck::Matches(_) => {}
        RegistrationTokenCheck::Missing => return Ok(ResolveExistingWaiter::Retry),
        RegistrationTokenCheck::Changed => {
            return Ok(ResolveExistingWaiter::Outcome(indeterminate(
                worker_id,
                Some(status),
                Some(path),
                "waiter registration changed before takeover".to_owned(),
            )));
        }
        RegistrationTokenCheck::Indeterminate { detail } => {
            return Ok(ResolveExistingWaiter::Outcome(indeterminate(
                worker_id,
                Some(status),
                Some(path),
                detail,
            )));
        }
    }

    log_waiter_takeover_requested(status, &waiter)?;
    if let Err(err) = process.terminate(waiter.pid) {
        return Ok(ResolveExistingWaiter::Outcome(takeover_failed(
            worker_id,
            status,
            path,
            &waiter,
            format!(
                "failed to send SIGTERM to waiter pid {}: {err:#}",
                waiter.pid
            ),
        )));
    }

    let deadline = Instant::now() + options.takeover_grace;
    loop {
        match process.is_running(waiter.pid) {
            Ok(false) => break,
            Ok(true) if Instant::now() < deadline => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                thread::sleep(TAKEOVER_POLL_INTERVAL.min(remaining));
            }
            Ok(true) => {
                return Ok(ResolveExistingWaiter::Outcome(takeover_failed(
                    worker_id,
                    status,
                    path,
                    &waiter,
                    format!(
                        "waiter pid {} still running after SIGTERM grace",
                        waiter.pid
                    ),
                )));
            }
            Err(err) => {
                return Ok(ResolveExistingWaiter::Outcome(takeover_failed(
                    worker_id,
                    status,
                    path,
                    &waiter,
                    format!(
                        "failed to confirm waiter pid {} exited: {err:#}",
                        waiter.pid
                    ),
                )));
            }
        }
    }

    match verify_registration_token(path, &waiter.token) {
        RegistrationTokenCheck::Matches(_) => {
            remove_waiter_file(path)?;
            log_waiter_taken_over(status, &waiter)?;
            Ok(ResolveExistingWaiter::Retry)
        }
        RegistrationTokenCheck::Missing => {
            log_waiter_taken_over(status, &waiter)?;
            Ok(ResolveExistingWaiter::Retry)
        }
        RegistrationTokenCheck::Changed => Ok(ResolveExistingWaiter::Outcome(indeterminate(
            worker_id,
            Some(status),
            Some(path),
            "waiter registration changed after takeover".to_owned(),
        ))),
        RegistrationTokenCheck::Indeterminate { detail } => Ok(ResolveExistingWaiter::Outcome(
            indeterminate(worker_id, Some(status), Some(path), detail),
        )),
    }
}

pub(in crate::wait) fn waiter_path(status: &Utf8Path) -> Utf8PathBuf {
    status.with_extension("waiter")
}

fn heartbeat_state(waiter: &WaiterRegistration, now: DateTime<Utc>) -> HeartbeatState {
    let (Some(heartbeat_at), Some(interval_ms)) =
        (waiter.heartbeat_at, waiter.heartbeat_interval_ms)
    else {
        return HeartbeatState::Missing;
    };
    let threshold = interval_ms.saturating_mul(3).max(MIN_STALE_HEARTBEAT_MS);
    let age = now.signed_duration_since(heartbeat_at).num_milliseconds();
    if age > u64_to_i64_saturating(threshold) {
        HeartbeatState::Stale
    } else {
        HeartbeatState::Fresh
    }
}

fn indeterminate(
    worker_id: Option<&str>,
    status: Option<&Utf8Path>,
    waiter: Option<&Utf8Path>,
    detail: String,
) -> WaitOutcome {
    WaitOutcome::Indeterminate(WaitIndeterminate {
        worker_id: worker_id.map(str::to_owned),
        status: status.map(Utf8Path::to_path_buf),
        waiter: waiter.map(Utf8Path::to_path_buf),
        detail,
    })
}

fn takeover_failed(
    worker_id: Option<&str>,
    status: &Utf8Path,
    path: &Utf8Path,
    waiter: &WaiterRegistration,
    detail: String,
) -> WaitOutcome {
    WaitOutcome::WaiterTakeoverFailed(WaiterTakeoverFailed {
        worker_id: worker_id.map(str::to_owned),
        status: status.to_path_buf(),
        waiter: path.to_path_buf(),
        holder_pid: waiter.pid,
        holder_since: waiter.started_at,
        detail,
    })
}

fn duration_millis(duration: Duration) -> u64 {
    u128_to_u64_saturating(duration.as_millis())
}

fn u64_to_i64_saturating(value: u64) -> i64 {
    if value > i64::MAX as u64 {
        i64::MAX
    } else {
        value as i64
    }
}

fn u128_to_u64_saturating(value: u128) -> u64 {
    if value > u128::from(u64::MAX) {
        u64::MAX
    } else {
        value as u64
    }
}

#[cfg(test)]
#[path = "guard_tests.rs"]
mod tests;
