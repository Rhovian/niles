use std::{
    cell::{Cell, RefCell},
    fs,
    time::Duration,
};

use anyhow::Result;
use camino::Utf8Path;
use chrono::{TimeDelta, Utc};

use super::*;
use crate::wait::{
    outcome::WaitOutcome,
    scanner::ack_log_path,
    test_support::{TestTempDir, test_worker_target, write_waiter_registration},
};

struct FakeProcess {
    running: Cell<bool>,
    terminate_stops: bool,
    terminate_calls: Cell<usize>,
    on_is_running: RefCell<Option<Box<dyn FnMut()>>>,
}

impl FakeProcess {
    fn new(running: bool) -> Self {
        Self {
            running: Cell::new(running),
            terminate_stops: true,
            terminate_calls: Cell::new(0),
            on_is_running: RefCell::new(None),
        }
    }

    fn terminate_stops(mut self, terminate_stops: bool) -> Self {
        self.terminate_stops = terminate_stops;
        self
    }

    fn on_is_running(mut self, action: impl FnMut() + 'static) -> Self {
        self.on_is_running = RefCell::new(Some(Box::new(action)));
        self
    }
}

impl WaiterProcess for FakeProcess {
    fn is_running(&self, _pid: u32) -> Result<bool> {
        if let Some(mut action) = self.on_is_running.borrow_mut().take() {
            action();
        }
        Ok(self.running.get())
    }

    fn terminate(&self, _pid: u32) -> Result<()> {
        self.terminate_calls.set(self.terminate_calls.get() + 1);
        if self.terminate_stops {
            self.running.set(false);
        }
        Ok(())
    }
}

#[test]
fn live_legacy_waiter_returns_typed_conflict() {
    let root = TestTempDir::new("legacy-conflict");
    let target = test_worker_target(root.path(), "auth-fix", "working: waiting\n");
    write_waiter_registration(
        target.status(),
        foreign_pid(1234),
        "legacy-token",
        Utc::now() - TimeDelta::minutes(1),
    );

    let attach = register_with_fake_process(&target, false, &FakeProcess::new(true));

    let WaiterAttach::Outcome(WaitOutcome::WaiterConflict(conflict)) = attach else {
        panic!("expected waiter conflict");
    };
    assert_eq!(conflict.worker_id.as_deref(), Some("auth-fix"));
    assert_eq!(conflict.holder_pid, foreign_pid(1234));
    assert_eq!(conflict.heartbeat, HeartbeatState::Missing);
    assert!(waiter_path(target.status()).exists());
}

#[test]
fn abandoned_legacy_waiter_is_reclaimed_instead_of_wedging() {
    let root = TestTempDir::new("legacy-aged-out");
    let target = test_worker_target(root.path(), "auth-fix", "working: waiting\n");
    write_waiter_registration(
        target.status(),
        foreign_pid(1234),
        "ancient-token",
        Utc::now() - TimeDelta::days(1),
    );

    // Live pid, no heartbeat fields: before aging this held the worker forever.
    let attach = register_with_fake_process(&target, false, &FakeProcess::new(true));

    let WaiterAttach::Attached(_guard) = attach else {
        panic!("expected an aged-out legacy registration to be reclaimed");
    };
    let ack_log = fs::read_to_string(ack_log_path(target.status())).unwrap();
    assert!(ack_log.contains(r#""reason":"heartbeat-stale""#));
    assert!(ack_log.contains(r#""token":"ancient-token""#));
}

#[test]
fn dead_waiter_reclaims_and_logs_stale_waiter() {
    let root = TestTempDir::new("dead-reclaim");
    let target = test_worker_target(root.path(), "auth-fix", "working: waiting\n");
    write_waiter_registration(target.status(), foreign_pid(4321), "dead-token", Utc::now());

    let attach = register_with_fake_process(&target, false, &FakeProcess::new(false));

    let WaiterAttach::Attached(_guard) = attach else {
        panic!("expected attached guard");
    };
    let ack_log = fs::read_to_string(ack_log_path(target.status())).unwrap();
    assert!(ack_log.contains(r#""event":"stale-waiter-reclaimed""#));
    assert!(ack_log.contains(r#""reason":"pid-dead""#));
    assert!(ack_log.contains(r#""token":"dead-token""#));
}

#[test]
fn pid_zero_is_reclaimed_and_never_signalled() {
    let root = TestTempDir::new("pid-zero");
    let target = test_worker_target(root.path(), "auth-fix", "working: waiting\n");
    // kill(0, SIGTERM) signals the caller's whole process group, so a takeover must never
    // reach the signal path with pid 0 no matter how fresh the registration claims to be.
    write_heartbeat_registration(target.status(), 0, "pid-zero-token", Utc::now(), 1000);
    let process = FakeProcess::new(true);

    let attach = register_with_fake_process(&target, true, &process);

    let WaiterAttach::Attached(_guard) = attach else {
        panic!("expected pid 0 registration to be reclaimed");
    };
    assert_eq!(
        process.terminate_calls.get(),
        0,
        "pid 0 must never be signalled"
    );
    let ack_log = fs::read_to_string(ack_log_path(target.status())).unwrap();
    assert!(ack_log.contains(r#""reason":"pid-zero""#));
}

#[test]
fn own_pid_is_reclaimed_as_a_recycled_leftover() {
    let root = TestTempDir::new("pid-self");
    let target = test_worker_target(root.path(), "auth-fix", "working: waiting\n");
    write_heartbeat_registration(
        target.status(),
        std::process::id(),
        "self-token",
        Utc::now(),
        1000,
    );
    let process = FakeProcess::new(true);

    let attach = register_with_fake_process(&target, true, &process);

    let WaiterAttach::Attached(_guard) = attach else {
        panic!("expected a registration holding our own pid to be reclaimed");
    };
    assert_eq!(
        process.terminate_calls.get(),
        0,
        "a waiter must never SIGTERM itself"
    );
    let ack_log = fs::read_to_string(ack_log_path(target.status())).unwrap();
    assert!(ack_log.contains(r#""reason":"pid-self""#));
}

#[test]
fn malformed_waiter_is_indeterminate_and_preserved() {
    let root = TestTempDir::new("malformed-waiter");
    let target = test_worker_target(root.path(), "auth-fix", "working: waiting\n");
    let path = waiter_path(target.status());
    fs::write(&path, "{not-json\n").unwrap();

    let attach = register_with_fake_process(&target, false, &FakeProcess::new(true));

    let WaiterAttach::Outcome(WaitOutcome::Indeterminate(indeterminate)) = attach else {
        panic!("expected indeterminate");
    };
    assert!(indeterminate.detail.contains("failed to parse"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "{not-json\n");
}

#[test]
fn heartbeat_stale_reclaims_and_fresh_conflicts() {
    let root = TestTempDir::new("heartbeat-state");
    let stale = test_worker_target(root.path(), "stale", "working: waiting\n");
    write_heartbeat_registration(
        stale.status(),
        foreign_pid(1111),
        "stale-token",
        Utc::now() - TimeDelta::seconds(20),
        1000,
    );

    let stale_attach = register_with_fake_process(&stale, false, &FakeProcess::new(true));
    let WaiterAttach::Attached(_guard) = stale_attach else {
        panic!("expected stale heartbeat reclaim");
    };
    let stale_ack_log = fs::read_to_string(ack_log_path(stale.status())).unwrap();
    assert!(stale_ack_log.contains(r#""reason":"heartbeat-stale""#));
    assert!(stale_ack_log.contains(r#""token":"stale-token""#));

    let fresh = test_worker_target(root.path(), "fresh", "working: waiting\n");
    write_heartbeat_registration(
        fresh.status(),
        foreign_pid(2222),
        "fresh-token",
        Utc::now(),
        1000,
    );

    let fresh_attach = register_with_fake_process(&fresh, false, &FakeProcess::new(true));
    let WaiterAttach::Outcome(WaitOutcome::WaiterConflict(conflict)) = fresh_attach else {
        panic!("expected fresh heartbeat conflict");
    };
    assert_eq!(conflict.heartbeat, HeartbeatState::Fresh);
    assert_eq!(conflict.holder_pid, foreign_pid(2222));
}

#[test]
fn takeover_rechecks_token_before_signalling() {
    let root = TestTempDir::new("takeover-recheck");
    let target = test_worker_target(root.path(), "auth-fix", "working: waiting\n");
    let path = waiter_path(target.status());
    write_heartbeat_registration(
        target.status(),
        foreign_pid(3333),
        "old-token",
        Utc::now(),
        1000,
    );
    let path_for_race = path.clone();
    let process = FakeProcess::new(true).on_is_running(move || {
        write_heartbeat_registration_at_path(
            &path_for_race,
            foreign_pid(4444),
            "new-token",
            Utc::now(),
            1000,
        );
    });

    let attach = register_with_fake_process(&target, true, &process);

    let WaiterAttach::Outcome(WaitOutcome::Indeterminate(indeterminate)) = attach else {
        panic!("expected indeterminate token race");
    };
    assert!(indeterminate.detail.contains("changed before takeover"));
    assert_eq!(process.terminate_calls.get(), 0);
    assert!(fs::read_to_string(&path).unwrap().contains("new-token"));
}

#[test]
fn takeover_refuses_when_holder_survives_grace() {
    let root = TestTempDir::new("takeover-refusal");
    let target = test_worker_target(root.path(), "auth-fix", "working: waiting\n");
    let path = waiter_path(target.status());
    write_heartbeat_registration(
        target.status(),
        foreign_pid(5555),
        "holder-token",
        Utc::now(),
        1000,
    );
    let process = FakeProcess::new(true).terminate_stops(false);

    let attach = register_with_fake_process(&target, true, &process);

    let WaiterAttach::Outcome(WaitOutcome::WaiterTakeoverFailed(failed)) = attach else {
        panic!("expected takeover failure");
    };
    assert_eq!(failed.holder_pid, foreign_pid(5555));
    assert!(failed.detail.contains("still running"));
    assert_eq!(process.terminate_calls.get(), 1);
    assert!(fs::read_to_string(&path).unwrap().contains("holder-token"));
    let ack_log = fs::read_to_string(ack_log_path(target.status())).unwrap();
    assert!(ack_log.contains(r#""event":"waiter-takeover-requested""#));
    assert!(!ack_log.contains(r#""event":"waiter-taken-over""#));
}

#[test]
fn takeover_logs_and_replaces_exited_holder() {
    let root = TestTempDir::new("takeover-success");
    let target = test_worker_target(root.path(), "auth-fix", "working: waiting\n");
    let path = waiter_path(target.status());
    write_heartbeat_registration(
        target.status(),
        foreign_pid(6666),
        "holder-token",
        Utc::now(),
        1000,
    );
    let process = FakeProcess::new(true);

    let attach = register_with_fake_process(&target, true, &process);

    let WaiterAttach::Attached(_guard) = attach else {
        panic!("expected takeover attach");
    };
    assert_eq!(process.terminate_calls.get(), 1);
    let waiter_body = fs::read_to_string(&path).unwrap();
    assert!(!waiter_body.contains("holder-token"));
    let ack_log = fs::read_to_string(ack_log_path(target.status())).unwrap();
    assert!(ack_log.contains(r#""event":"waiter-takeover-requested""#));
    assert!(ack_log.contains(r#""event":"waiter-taken-over""#));
}

/// Keeps a test pid distinct from this process's own, which now carries reclaim semantics.
fn foreign_pid(seed: u32) -> u32 {
    if seed == std::process::id() {
        seed.wrapping_add(1)
    } else {
        seed
    }
}

fn register_with_fake_process(
    target: &crate::wait::target::WaitTarget,
    takeover: bool,
    process: &FakeProcess,
) -> WaiterAttach {
    let options =
        WaiterOptions::new(takeover, Duration::from_millis(50)).with_takeover_grace(Duration::ZERO);
    WaiterGuard::register_with_process(target.status(), target.worker_id(), &options, process)
        .unwrap()
}

fn write_heartbeat_registration(
    status: &Utf8Path,
    pid: u32,
    token: &str,
    heartbeat_at: chrono::DateTime<Utc>,
    heartbeat_interval_ms: u64,
) {
    write_heartbeat_registration_at_path(
        &waiter_path(status),
        pid,
        token,
        heartbeat_at,
        heartbeat_interval_ms,
    );
}

fn write_heartbeat_registration_at_path(
    path: &Utf8Path,
    pid: u32,
    token: &str,
    heartbeat_at: chrono::DateTime<Utc>,
    heartbeat_interval_ms: u64,
) {
    fs::write(
        path,
        format!(
            r#"{{
  "pid": {pid},
  "started_at": "2000-01-01T00:00:00Z",
  "token": "{token}",
  "heartbeat_at": "{}",
  "heartbeat_interval_ms": {heartbeat_interval_ms}
}}
"#,
            heartbeat_at.to_rfc3339()
        ),
    )
    .unwrap();
}
