mod common;

use common::*;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn wait_worker_returns_unconsumed_wake_already_in_status() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-wait-preexisting");
    let worker_dir = workspace.join(".niles/worker/auth-fix");
    fs::create_dir_all(&worker_dir).unwrap();
    let status_log = worker_dir.join("status.log");
    fs::write(&status_log, "done: already complete\n").unwrap();

    let started = Instant::now();
    let output = Command::new(niles)
        .args([
            "wait",
            "--worker",
            "auth-fix",
            "--interval",
            "0.05",
            "--timeout",
            "0",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "wait did not return promptly; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_command_success("wait --worker preexisting", &output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "done: already complete\n"
    );
    assert_eq!(
        fs::read_to_string(worker_dir.join("status.ack")).unwrap(),
        "1\n"
    );
}

#[test]
fn wait_worker_does_not_redeliver_consumed_wake_and_delivers_next() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-wait-ack");
    let worker_dir = workspace.join(".niles/worker/auth-fix");
    fs::create_dir_all(&worker_dir).unwrap();
    let status_log = worker_dir.join("status.log");
    fs::write(&status_log, "done: first\n").unwrap();

    let first = Command::new(niles)
        .args([
            "wait",
            "--worker",
            "auth-fix",
            "--interval",
            "0.05",
            "--timeout",
            "0",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert_command_success("first wait --worker", &first);
    assert_eq!(String::from_utf8_lossy(&first.stdout), "done: first\n");

    let second = Command::new(niles)
        .args([
            "wait",
            "--worker",
            "auth-fix",
            "--interval",
            "0.05",
            "--timeout",
            "0",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stdout).is_empty());
    assert!(String::from_utf8_lossy(&second.stderr).contains("timeout"));

    let mut status = fs::OpenOptions::new()
        .append(true)
        .open(&status_log)
        .unwrap();
    writeln!(status, "done: second").unwrap();

    let third = Command::new(niles)
        .args([
            "wait",
            "--worker",
            "auth-fix",
            "--interval",
            "0.05",
            "--timeout",
            "0",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert_command_success("third wait --worker", &third);
    assert_eq!(String::from_utf8_lossy(&third.stdout), "done: second\n");
    assert_eq!(
        fs::read_to_string(worker_dir.join("status.ack")).unwrap(),
        "2\n"
    );
}

#[test]
fn wait_worker_second_sequential_wait_returns_followup_wake() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-wait-followup");
    let worker_dir = workspace.join(".niles/worker/auth-fix");
    fs::create_dir_all(&worker_dir).unwrap();
    let status_log = worker_dir.join("status.log");
    fs::write(&status_log, "done: first result\n").unwrap();

    let first = Command::new(niles)
        .args([
            "wait",
            "--worker",
            "auth-fix",
            "--interval",
            "0.05",
            "--timeout",
            "0",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert_command_success("first sequential wait --worker", &first);
    assert_eq!(
        String::from_utf8_lossy(&first.stdout),
        "done: first result\n"
    );

    let mut status = fs::OpenOptions::new()
        .append(true)
        .open(&status_log)
        .unwrap();
    writeln!(status, "done: follow-up result").unwrap();

    let second = Command::new(niles)
        .args([
            "wait",
            "--worker",
            "auth-fix",
            "--interval",
            "0.05",
            "--timeout",
            "0",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert_command_success("second sequential wait --worker", &second);
    assert_eq!(
        String::from_utf8_lossy(&second.stdout),
        "done: follow-up result\n"
    );
    assert!(!worker_dir.join("status.waiter").exists());
    assert_eq!(
        fs::read_to_string(worker_dir.join("status.ack")).unwrap(),
        "2\n"
    );
}

#[test]
fn wait_worker_rejects_second_unindexed_wait_while_first_is_attached() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-wait-duplicate");
    let worker_dir = workspace.join(".niles/worker/auth-fix");
    fs::create_dir_all(&worker_dir).unwrap();
    let status_log = worker_dir.join("status.log");
    fs::write(&status_log, "working: first waiter is attached\n").unwrap();

    let first = Command::new(niles)
        .args([
            "wait",
            "--worker",
            "auth-fix",
            "--interval",
            "0.05",
            "--timeout",
            "5",
        ])
        .current_dir(&workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    wait_for_path(&worker_dir.join("status.waiter"));

    let second = Command::new(niles)
        .args([
            "wait",
            "--worker",
            "auth-fix",
            "--interval",
            "0.05",
            "--timeout",
            "5",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stdout).is_empty());
    let second_stderr = String::from_utf8_lossy(&second.stderr);
    assert!(second_stderr.contains("another unindexed wait is already attached"));
    assert!(second_stderr.contains("pid"));
    assert!(second_stderr.contains("is running"));
    assert!(second_stderr.contains("status.waiter"));

    let mut status = fs::OpenOptions::new()
        .append(true)
        .open(&status_log)
        .unwrap();
    writeln!(status, "done: first waiter wake").unwrap();

    let first = first.wait_with_output().unwrap();
    assert_command_success("first wait --worker duplicate", &first);
    assert_eq!(
        String::from_utf8_lossy(&first.stdout),
        "done: first waiter wake\n"
    );
    assert!(!worker_dir.join("status.waiter").exists());

    let ack_log = fs::read_to_string(worker_dir.join("status.ack.log")).unwrap();
    assert!(ack_log.contains(r#""event":"wake-consumed""#));
    assert!(ack_log.contains(r#""line":"done: first waiter wake""#));
    assert!(ack_log.contains(r#""pid":"#));
}

#[test]
fn wait_worker_bails_when_guard_is_removed_mid_wait() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-wait-guard-removed");
    let worker_dir = workspace.join(".niles/worker/auth-fix");
    fs::create_dir_all(&worker_dir).unwrap();
    fs::write(worker_dir.join("status.log"), "working: waiting\n").unwrap();

    let waiter = Command::new(niles)
        .args([
            "wait",
            "--worker",
            "auth-fix",
            "--interval",
            "0.05",
            "--timeout",
            "5",
        ])
        .current_dir(&workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let guard = worker_dir.join("status.waiter");
    wait_for_path(&guard);
    fs::remove_file(&guard).unwrap();

    let output = waiter.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("waiter registration was removed/replaced while waiting"));
    assert!(!stderr.contains("timeout"));
}

#[test]
fn wait_worker_reclaims_dead_waiter_registration() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-wait-dead-guard");
    let worker_dir = workspace.join(".niles/worker/auth-fix");
    fs::create_dir_all(&worker_dir).unwrap();
    fs::write(worker_dir.join("status.log"), "done: after stale waiter\n").unwrap();
    write_waiter_registration(&worker_dir, i32::MAX as u32, "stale-token");

    let output = Command::new(niles)
        .args([
            "wait",
            "--worker",
            "auth-fix",
            "--interval",
            "0.05",
            "--timeout",
            "0",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert_command_success("wait --worker dead waiter reclaim", &output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "done: after stale waiter\n"
    );
    assert!(!worker_dir.join("status.waiter").exists());

    let ack_log = fs::read_to_string(worker_dir.join("status.ack.log")).unwrap();
    assert!(ack_log.contains(r#""event":"stale-waiter-reclaimed""#));
    assert!(ack_log.contains(r#""pid":2147483647"#));
    assert!(ack_log.contains(r#""token":"stale-token""#));
    assert!(ack_log.contains(r#""event":"wake-consumed""#));
}

#[test]
fn wait_worker_cleans_guard_on_timeout() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-wait-timeout-cleanup");
    let worker_dir = workspace.join(".niles/worker/auth-fix");
    fs::create_dir_all(&worker_dir).unwrap();
    fs::write(worker_dir.join("status.log"), "working: no wake yet\n").unwrap();

    let output = Command::new(niles)
        .args([
            "wait",
            "--worker",
            "auth-fix",
            "--interval",
            "0.05",
            "--timeout",
            "0",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("timeout"));
    assert!(!worker_dir.join("status.waiter").exists());
}

#[test]
fn wait_worker_corrupt_ack_fails_loudly() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-wait-corrupt-ack");
    let worker_dir = workspace.join(".niles/worker/auth-fix");
    fs::create_dir_all(&worker_dir).unwrap();
    fs::write(worker_dir.join("status.log"), "done: after corrupt ack\n").unwrap();
    fs::write(worker_dir.join("status.ack"), "not a number\n").unwrap();

    let output = Command::new(niles)
        .args([
            "wait",
            "--worker",
            "auth-fix",
            "--interval",
            "0.05",
            "--timeout",
            "0",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid ack cursor"));
    assert!(stderr.contains("invalid digit found in string"));
    assert_eq!(
        fs::read_to_string(worker_dir.join("status.ack")).unwrap(),
        "not a number\n"
    );
}

#[test]
fn wait_worker_unknown_id_errors_without_closed_backstop() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-wait-unknown");

    let output = Command::new(niles)
        .args([
            "wait",
            "--worker",
            "missing",
            "--interval",
            "0.05",
            "--timeout",
            "0",
        ])
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown worker id 'missing'"));
    assert!(!stderr.contains("worker 'missing' closed"));
}

#[test]
fn wait_worker_returns_closed_backstop_when_resolved_directory_is_removed() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-wait-removed");
    let worker_root = workspace.join(".niles/worker");
    let worker_dir = worker_root.join("auth-fix");
    fs::create_dir_all(&worker_dir).unwrap();
    fs::write(worker_dir.join("status.log"), "working: still running\n").unwrap();

    let waiter = Command::new(niles)
        .args([
            "wait",
            "--worker",
            "auth-fix",
            "--interval",
            "0.05",
            "--timeout",
            "5",
        ])
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(100));
    fs::remove_dir_all(&worker_dir).unwrap();

    let output = waiter.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "closed: worker 'auth-fix' directory removed\n"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("worker 'auth-fix' closed"));
    assert!(!stderr.contains("timeout"));
}

fn wait_for_path(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(path.exists(), "{} did not appear", path.display());
}

fn write_waiter_registration(dir: &Path, pid: u32, token: &str) {
    fs::write(
        dir.join("status.waiter"),
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
