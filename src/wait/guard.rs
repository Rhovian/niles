use std::{
    fs,
    io::{ErrorKind, Write},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::util::{append_line, read_optional_to_string, timestamp_id};

use super::scanner::ack_log_path;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(in crate::wait) struct WaiterRegistration {
    pub(in crate::wait) pid: u32,
    pub(in crate::wait) started_at: DateTime<Utc>,
    pub(in crate::wait) token: String,
}

pub(in crate::wait) struct WaiterGuard {
    path: Utf8PathBuf,
    pub(in crate::wait) registration: WaiterRegistration,
}

impl WaiterGuard {
    pub(in crate::wait) fn register(status: &Utf8Path) -> Result<Self> {
        let path = waiter_path(status);
        let started_at = Utc::now();
        let pid = std::process::id();
        let registration = WaiterRegistration {
            pid,
            started_at,
            token: format!("{pid}-{}", timestamp_id(&started_at)),
        };
        let body = format!("{}\n", serde_json::to_string_pretty(&registration)?);

        for retry in 0..=1 {
            match Self::try_create(&path, registration.clone(), &body)? {
                Some(guard) => return Ok(guard),
                None => {
                    resolve_existing_waiter(status, &path)?;
                    if retry == 1 {
                        bail!(
                            "waiter registration changed while attaching to {status}; retry wait"
                        );
                    }
                }
            }
        }

        unreachable!()
    }

    fn try_create(
        path: &Utf8Path,
        registration: WaiterRegistration,
        body: &str,
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
                }))
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => Ok(None),
            Err(err) => Err(err).with_context(|| format!("failed to create {path}")),
        }
    }

    pub(in crate::wait) fn verify(&self) -> Result<()> {
        let registration = read_optional_json::<WaiterRegistration>(
            &self.path,
            |path| format!("failed to read {path}"),
            |path| format!("failed to parse {path}"),
        )
        .with_context(|| {
            format!(
                "waiter registration was removed/replaced while waiting: {}",
                self.path
            )
        })?;

        if registration
            .as_ref()
            .is_some_and(|registration| registration.token == self.registration.token)
        {
            return Ok(());
        }

        bail!(
            "waiter registration was removed/replaced while waiting: {}",
            self.path
        )
    }
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        let Ok(Some(registration)) = read_optional_json::<WaiterRegistration>(
            &self.path,
            |_| String::new(),
            |_| String::new(),
        ) else {
            return;
        };

        if registration.token == self.registration.token {
            let _ = fs::remove_file(&self.path);
        }
    }
}

// The waiter guard is ephemeral wake-protocol state (like status.ack), not a
// durable stamped artifact class, so it is read with a plain JSON reader
// rather than schema::read_optional_json.
fn read_optional_json<T>(
    path: &Utf8Path,
    read_context: impl FnOnce(&Utf8Path) -> String,
    parse_context: impl FnOnce(&Utf8Path) -> String,
) -> Result<Option<T>>
where
    T: serde::de::DeserializeOwned,
{
    let Some(body) = read_optional_to_string(path, read_context)? else {
        return Ok(None);
    };

    serde_json::from_str(&body)
        .map(Some)
        .with_context(|| parse_context(path))
}

fn resolve_existing_waiter(status: &Utf8Path, path: &Utf8Path) -> Result<()> {
    match read_optional_json::<WaiterRegistration>(
        path,
        |path| format!("failed to read {path}"),
        |path| format!("failed to parse {path}"),
    ) {
        Ok(Some(waiter)) if process_is_running(waiter.pid)? => {
            bail!(
                "another unindexed wait is already attached to {status} (pid {}, since {}; pid {} is running); \
                 active waiter registration: {path}",
                waiter.pid,
                waiter.started_at.to_rfc3339(),
                waiter.pid
            )
        }
        Ok(Some(waiter)) => {
            log_stale_waiter_reclaimed(status, &waiter)?;
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(err) if err.kind() == ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("failed to remove stale waiter registration {path}")
                    });
                }
            }
            Ok(())
        }
        Ok(None) => Ok(()),
        Err(err) => bail!(
            "another unindexed wait appears to be attached to {status}, but {path} could not be read: {err}; \
             remove that file if the waiter is stale"
        ),
    }
}

pub(in crate::wait) fn waiter_path(status: &Utf8Path) -> Utf8PathBuf {
    status.with_extension("waiter")
}

fn log_stale_waiter_reclaimed(status: &Utf8Path, waiter: &WaiterRegistration) -> Result<()> {
    let path = ack_log_path(status);
    let record = serde_json::json!({
        "event": "stale-waiter-reclaimed",
        "pid": waiter.pid,
        "started_at": waiter.started_at.to_rfc3339(),
        "token": &waiter.token,
        "reclaimed_at": Utc::now().to_rfc3339(),
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

fn process_is_running(pid: u32) -> Result<bool> {
    let pid = libc::pid_t::try_from(pid)
        .with_context(|| format!("waiter pid {pid} does not fit platform pid_t"))?;
    // SAFETY: signal 0 performs permission/existence checks without delivering a signal.
    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        return Ok(true);
    }

    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(code) if code == libc::ESRCH => Ok(false),
        Some(code) if code == libc::EPERM => Ok(true),
        _ => Err(err).with_context(|| format!("failed to check whether pid {pid} is running")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wait::{
        scanner::ack_log_path,
        test_support::{TestTempDir, test_worker_target, write_waiter_registration},
        wait_for_first_wake,
    };
    use std::{fs, time::Duration};

    #[test]
    fn acquire_all_release_all_on_conflict() {
        let root = TestTempDir::new("guard-conflict");
        let first = test_worker_target(root.path(), "first", "working: first\n");
        let first_waiter = waiter_path(first.status());
        let second = test_worker_target(root.path(), "second", "working: second\n");
        let second_waiter = waiter_path(second.status());
        write_waiter_registration(second.status(), std::process::id(), "live-token");
        let third = test_worker_target(root.path(), "third", "working: third\n");
        let third_waiter = waiter_path(third.status());

        let err = wait_for_first_wake(
            vec![first, second, third],
            Duration::from_millis(1),
            Duration::ZERO,
        )
        .err()
        .unwrap();

        assert!(format!("{err:#}").contains("another unindexed wait is already attached"));
        assert!(!first_waiter.exists());
        assert!(second_waiter.exists());
        assert!(!third_waiter.exists());
    }

    #[test]
    fn release_all_on_fire() {
        let root = TestTempDir::new("release-on-fire");
        let first = test_worker_target(root.path(), "first", "done: fire\n");
        let first_waiter = waiter_path(first.status());
        let second = test_worker_target(root.path(), "second", "working: waiting\n");
        let second_waiter = waiter_path(second.status());

        let wake = wait_for_first_wake(
            vec![first, second],
            Duration::from_millis(1),
            Duration::ZERO,
        )
        .unwrap();

        assert_eq!(wake.worker_id.as_deref(), Some("first"));
        assert!(!first_waiter.exists());
        assert!(!second_waiter.exists());
    }

    #[test]
    fn stale_guard_reclaimed_per_target() {
        let root = TestTempDir::new("stale-guard");
        let first = test_worker_target(root.path(), "first", "working: first\n");
        write_waiter_registration(first.status(), i32::MAX as u32, "stale-token");
        let first_waiter = waiter_path(first.status());
        let first_ack_log = ack_log_path(first.status());
        let second = test_worker_target(root.path(), "second", "done: second\n");

        let wake = wait_for_first_wake(
            vec![first, second],
            Duration::from_millis(1),
            Duration::ZERO,
        )
        .unwrap();

        assert_eq!(wake.worker_id.as_deref(), Some("second"));
        assert!(!first_waiter.exists());
        let ack_log = fs::read_to_string(first_ack_log).unwrap();
        assert!(ack_log.contains(r#""event":"stale-waiter-reclaimed""#));
        assert!(ack_log.contains(r#""token":"stale-token""#));
    }
}
