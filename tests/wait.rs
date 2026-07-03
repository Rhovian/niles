mod common;

use common::*;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[test]
fn wait_skips_acknowledged_status_lines_and_prints_next_wake() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-wait-test");

    let run_dir = workspace.join(".niles/runs/test-run");
    fs::create_dir_all(&run_dir).unwrap();
    let status_log = run_dir.join("status.log");
    fs::write(
        &status_log,
        "done: old baseline wake\nworking: already running\n",
    )
    .unwrap();
    fs::write(run_dir.join("status.ack"), "1\n").unwrap();

    let child = Command::new(niles)
        .args(["wait", "test-run", "--interval", "0.05", "--timeout", "5"])
        .current_dir(&workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let writer_done = Arc::new(AtomicBool::new(false));
    let writer_status = status_log.clone();
    let writer_done_clone = Arc::clone(&writer_done);
    let writer = thread::spawn(move || {
        while !writer_done_clone.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(50));
            let mut status = fs::OpenOptions::new()
                .append(true)
                .open(&writer_status)
                .unwrap();
            writeln!(status, "done: new appended wake").unwrap();
        }
    });

    let output = child.wait_with_output().unwrap();
    writer_done.store(true, Ordering::SeqCst);
    writer.join().unwrap();

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "done: new appended wake\n"
    );
}

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
fn wait_run_rejects_second_unindexed_wait_while_first_is_attached() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-run-wait-duplicate");
    let run_dir = workspace.join(".niles/runs/test-run");
    fs::create_dir_all(&run_dir).unwrap();
    let status_log = run_dir.join("status.log");
    fs::write(&status_log, "working: first waiter is attached\n").unwrap();

    let first = Command::new(niles)
        .args(["wait", "test-run", "--interval", "0.05", "--timeout", "5"])
        .current_dir(&workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    wait_for_path(&run_dir.join("status.waiter"));

    let second = Command::new(niles)
        .args(["wait", "test-run", "--interval", "0.05", "--timeout", "5"])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stdout).is_empty());
    let second_stderr = String::from_utf8_lossy(&second.stderr);
    assert!(second_stderr.contains("another unindexed wait is already attached"));
    assert!(second_stderr.contains("pid"));
    assert!(second_stderr.contains("status.waiter"));

    let mut status = fs::OpenOptions::new()
        .append(true)
        .open(&status_log)
        .unwrap();
    writeln!(status, "done: run waiter wake").unwrap();

    let first = first.wait_with_output().unwrap();
    assert_command_success("first wait run duplicate", &first);
    assert_eq!(
        String::from_utf8_lossy(&first.stdout),
        "done: run waiter wake\n"
    );
    assert!(!run_dir.join("status.waiter").exists());
}

#[test]
fn wait_worker_corrupt_ack_falls_back_to_start() {
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

    assert_command_success("wait --worker corrupt ack", &output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "done: after corrupt ack\n"
    );
    assert_eq!(
        fs::read_to_string(worker_dir.join("status.ack")).unwrap(),
        "1\n"
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
    write_worker_pointer(&workspace, "auth-fix", &worker_dir);

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

#[test]
fn wait_index_returns_already_emitted_step_wake() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-wait-index-test");

    let run_dir = workspace.join(".niles/runs/test-run");
    fs::create_dir_all(&run_dir).unwrap();
    fs::write(
        run_dir.join("status.log"),
        "done: step 1 finished\nworking: step 2 running\ndone: step 2 finished\n",
    )
    .unwrap();

    let started = Instant::now();
    let output = Command::new(niles)
        .args([
            "wait",
            "test-run",
            "--index",
            "2",
            "--interval",
            "0.05",
            "--timeout",
            "4",
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
    assert_command_success("wait --index", &output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "done: step 2 finished\n"
    );
}

#[test]
fn wait_index_accepts_untagged_wake_attributed_to_running_step() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-wait-index-attributed-test");

    let run_dir = workspace.join(".niles/runs/test-run");
    fs::create_dir_all(&run_dir).unwrap();
    write_manual_run_state(&run_dir, "test-run", 2);
    fs::write(
        run_dir.join("status.log"),
        "done: CONSENSUS - reviewer approved the change\n",
    )
    .unwrap();

    let started = Instant::now();
    let output = Command::new(niles)
        .args([
            "wait",
            "test-run",
            "--index",
            "2",
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
    assert_command_success("wait --index attributed", &output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "done: CONSENSUS - reviewer approved the change\n"
    );
}

#[test]
fn wait_index_rejects_untagged_wake_attributed_to_another_step() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-wait-index-other-attributed-test");

    let run_dir = workspace.join(".niles/runs/test-run");
    fs::create_dir_all(&run_dir).unwrap();
    write_manual_run_state(&run_dir, "test-run", 1);
    fs::write(
        run_dir.join("status.log"),
        "done: CONSENSUS - reviewer approved the change\n",
    )
    .unwrap();

    let output = Command::new(niles)
        .args([
            "wait",
            "test-run",
            "--index",
            "2",
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
}

fn wait_for_path(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(path.exists(), "{} did not appear", path.display());
}

fn write_manual_run_state(run_dir: &Path, id: &str, running_step: usize) {
    let step1_status = if running_step == 1 {
        "running"
    } else {
        "completed"
    };
    let step2_status = if running_step == 2 {
        "running"
    } else {
        "pending"
    };
    fs::write(
        run_dir.join("state.json"),
        format!(
            r#"{{
  "niles_schema": 2,
  "id": "{id}",
  "goal": "manual wait test",
  "created_at": "2000-01-01T00:00:00Z",
  "updated_at": "2000-01-01T00:00:00Z",
  "status": "running",
  "steps": [
    {{
      "index": 1,
      "kind": "agent",
      "label": "one",
      "status": "{step1_status}",
      "started_at": "2000-01-01T00:00:00Z"
    }},
    {{
      "index": 2,
      "kind": "agent",
      "label": "two",
      "status": "{step2_status}",
      "started_at": "2000-01-01T00:01:00Z"
    }}
  ]
}}
"#,
        ),
    )
    .unwrap();
}

fn write_worker_pointer(workspace: &Path, id: &str, worker_dir: &Path) {
    let worker_root = workspace.join(".niles/worker");
    fs::create_dir_all(&worker_root).unwrap();
    fs::write(
        worker_root.join(format!("{id}.json")),
        format!(
            r#"{{
  "niles_schema": 2,
  "id": "{id}",
  "workspace": "{}",
  "worker_dir": "{}",
  "local_stores": ["{}"]
}}
"#,
            workspace.display(),
            worker_dir.display(),
            worker_root.display()
        ),
    )
    .unwrap();
}
