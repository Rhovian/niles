mod common;

use common::*;
use std::{
    fs,
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Starts a wait and returns the child. Callers that need the wait to already be polling before
/// they perturb the worker use [`settle`] — with the waiter-registration file gone there is no
/// artifact to synchronise on, and the wait has nothing to race against anyway.
fn spawn_wait(workspace: &Path, args: &[&str]) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_niles"))
        .arg("wait")
        .args(args)
        .current_dir(workspace)
        .env("NILES_HOME", niles_home(workspace))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn run_wait(workspace: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_niles"))
        .arg("wait")
        .args(args)
        .current_dir(workspace)
        .env("NILES_HOME", niles_home(workspace))
        .output()
        .unwrap()
}

fn settle() {
    thread::sleep(Duration::from_millis(300));
}

fn worker_with_status(workspace: &Path, id: &str, status: &[u8]) -> std::path::PathBuf {
    let worker_dir = workspace.join(".niles/worker").join(id);
    fs::create_dir_all(&worker_dir).unwrap();
    fs::write(worker_dir.join("status.log"), status).unwrap();
    worker_dir
}

fn cursor(worker_dir: &Path) -> String {
    fs::read_to_string(worker_dir.join("status.cursor")).unwrap()
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn returns_unconsumed_wake_already_in_status() {
    let workspace = temp_workspace("niles-wait-preexisting");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"done: already complete\n");

    let started = Instant::now();
    let output = run_wait(
        &workspace,
        &["auth-fix", "--interval", "0.05", "--timeout", "0"],
    );

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "wait did not return promptly; stdout:\n{}\nstderr:\n{}",
        stdout_of(&output),
        stderr_of(&output)
    );
    assert_command_success("wait --worker preexisting", &output);
    assert_eq!(stdout_of(&output), "done: already complete\n");
    // The cursor is a byte offset just past the delivered line, here the whole file.
    assert_eq!(cursor(&worker_dir), "23\n");
}

#[test]
fn does_not_redeliver_consumed_wake_and_delivers_next() {
    let workspace = temp_workspace("niles-wait-cursor");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"done: first\n");
    let args = ["auth-fix", "--interval", "0.05", "--timeout", "0"];

    let first = run_wait(&workspace, &args);
    assert_command_success("first wait", &first);
    assert_eq!(stdout_of(&first), "done: first\n");
    assert_eq!(cursor(&worker_dir), "12\n");

    let second = run_wait(&workspace, &args);
    assert!(!second.status.success());
    assert_eq!(second.status.code(), Some(22));
    assert!(stdout_of(&second).is_empty());
    assert!(stderr_of(&second).contains("timeout"));

    let mut status = fs::OpenOptions::new()
        .append(true)
        .open(worker_dir.join("status.log"))
        .unwrap();
    writeln!(status, "done: second").unwrap();

    let third = run_wait(&workspace, &args);
    assert_command_success("third wait", &third);
    assert_eq!(stdout_of(&third), "done: second\n");
    assert_eq!(cursor(&worker_dir), "25\n");
}

#[test]
fn skips_non_actionable_lines_without_persisting_past_an_undelivered_wake() {
    let workspace = temp_workspace("niles-wait-working");
    let worker_dir = worker_with_status(
        &workspace,
        "auth-fix",
        b"working: one\nworking: two\nneeds-decision: pick a lane\nworking: three\n",
    );

    let output = run_wait(
        &workspace,
        &["auth-fix", "--interval", "0.05", "--timeout", "0"],
    );

    assert_command_success("wait past working lines", &output);
    assert_eq!(stdout_of(&output), "needs-decision: pick a lane\n");
    // Stops just past the decision line, not at EOF: the trailing `working:` stays unscanned.
    assert_eq!(cursor(&worker_dir), "54\n");
}

#[test]
fn leaves_an_unterminated_trailing_line_for_the_next_poll() {
    let workspace = temp_workspace("niles-wait-partial");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"done: comp");

    let partial = run_wait(
        &workspace,
        &["auth-fix", "--interval", "0.05", "--timeout", "0"],
    );
    assert_eq!(partial.status.code(), Some(22));
    // The cursor file exists because it doubles as the lock, but holds no advanced position.
    let recorded = cursor(&worker_dir);
    assert!(
        recorded.trim().is_empty() || recorded.trim() == "0",
        "a half-written line must not advance the cursor, found {recorded:?}"
    );

    let mut status = fs::OpenOptions::new()
        .append(true)
        .open(worker_dir.join("status.log"))
        .unwrap();
    status.write_all(b"lete\n").unwrap();

    let whole = run_wait(
        &workspace,
        &["auth-fix", "--interval", "0.05", "--timeout", "0"],
    );
    assert_command_success("wait after line completed", &whole);
    assert_eq!(stdout_of(&whole), "done: complete\n");
}

#[test]
fn escapes_control_characters_instead_of_emitting_them() {
    let workspace = temp_workspace("niles-wait-control");
    worker_with_status(
        &workspace,
        "auth-fix",
        b"done: shipped\x1b[2J\x1b[H\x07 and cleared your screen\n",
    );

    let output = run_wait(
        &workspace,
        &["auth-fix", "--interval", "0.05", "--timeout", "0"],
    );

    assert_command_success("wait with control characters", &output);
    let stdout = stdout_of(&output);
    assert!(!stdout.contains('\x1b'), "raw escape reached stdout: {stdout:?}");
    assert!(!stdout.contains('\x07'), "raw bell reached stdout: {stdout:?}");
    assert!(stdout.contains("shipped"));
    assert!(stdout.contains("and cleared your screen"));
}

#[test]
fn non_utf8_bytes_do_not_desynchronise_the_cursor() {
    let workspace = temp_workspace("niles-wait-binary");
    let mut status = b"working: \xff\xfe garbage\n".to_vec();
    status.extend_from_slice(b"done: recovered\n");
    let worker_dir = worker_with_status(&workspace, "auth-fix", &status);
    let total = fs::metadata(worker_dir.join("status.log")).unwrap().len();

    let output = run_wait(
        &workspace,
        &["auth-fix", "--interval", "0.05", "--timeout", "0"],
    );

    assert_command_success("wait past non-utf8", &output);
    assert_eq!(stdout_of(&output), "done: recovered\n");
    // Offsets come from raw bytes, so lossy decoding of the garbage line cannot shift them.
    assert_eq!(cursor(&worker_dir), format!("{total}\n"));
}

#[test]
fn a_truncated_log_rescans_from_the_start() {
    let workspace = temp_workspace("niles-wait-truncated");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"done: first\n");
    let args = ["auth-fix", "--interval", "0.05", "--timeout", "0"];

    assert_command_success("first wait", &run_wait(&workspace, &args));
    assert_eq!(cursor(&worker_dir), "12\n");

    // Rewrite the log shorter than the recorded offset of 12.
    fs::write(worker_dir.join("status.log"), b"done: new\n").unwrap();

    let output = run_wait(&workspace, &args);
    assert_command_success("wait after truncation", &output);
    assert_eq!(stdout_of(&output), "done: new\n");
    assert_eq!(cursor(&worker_dir), "10\n");
}

#[test]
fn concurrent_waits_deliver_the_line_to_exactly_one() {
    let workspace = temp_workspace("niles-wait-concurrent");
    worker_with_status(&workspace, "auth-fix", b"working: still running\n");
    let args = ["auth-fix", "--interval", "0.05", "--timeout", "5"];

    let first = spawn_wait(&workspace, &args);
    let second = spawn_wait(&workspace, &args);
    settle();

    let mut status = fs::OpenOptions::new()
        .append(true)
        .open(workspace.join(".niles/worker/auth-fix/status.log"))
        .unwrap();
    writeln!(status, "done: only once").unwrap();

    let first = first.wait_with_output().unwrap();
    let second = second.wait_with_output().unwrap();

    // Neither wait is rejected for the other's existence; exactly one is handed the line.
    let winners = [&first, &second]
        .into_iter()
        .filter(|output| stdout_of(output).contains("done: only once"))
        .count();
    assert_eq!(
        winners,
        1,
        "first: {:?}/{:?}\nsecond: {:?}/{:?}",
        stdout_of(&first),
        stderr_of(&first),
        stdout_of(&second),
        stderr_of(&second)
    );
    let losers = [&first, &second]
        .into_iter()
        .filter(|output| output.status.code() == Some(22))
        .count();
    assert_eq!(losers, 1, "the wait that lost the race should time out");
}

#[test]
fn waiting_on_several_workers_prefixes_the_winning_id() {
    let workspace = temp_workspace("niles-wait-fleet");
    worker_with_status(&workspace, "alpha", b"working: nothing yet\n");
    worker_with_status(&workspace, "beta", b"done: beta finished\n");

    let output = run_wait(
        &workspace,
        &[
            "alpha", "beta", "--interval", "0.05", "--timeout", "0",
        ],
    );

    assert_command_success("fleet wait", &output);
    assert_eq!(stdout_of(&output), "beta: done: beta finished\n");
}

#[test]
fn corrupt_cursor_fails_loudly_and_names_the_file() {
    let workspace = temp_workspace("niles-wait-corrupt");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"done: ready\n");
    fs::write(worker_dir.join("status.cursor"), "not-a-number\n").unwrap();

    let output = run_wait(
        &workspace,
        &["auth-fix", "--interval", "0.05", "--timeout", "0"],
    );

    assert!(!output.status.success());
    let stderr = stderr_of(&output);
    assert!(stderr.contains("invalid wake cursor"), "stderr: {stderr}");
    // The operator has to be told which file to delete, or the worker is wedged.
    assert!(stderr.contains("status.cursor"), "stderr: {stderr}");
    assert!(stderr.contains("remove it to resume"), "stderr: {stderr}");
}

#[test]
fn unknown_id_errors_without_closed_backstop() {
    let workspace = temp_workspace("niles-wait-unknown");

    let output = run_wait(
        &workspace,
        &["missing", "--interval", "0.05", "--timeout", "0"],
    );

    assert!(!output.status.success());
    assert!(stdout_of(&output).is_empty());
    let stderr = stderr_of(&output);
    assert!(stderr.contains("unknown worker id 'missing'"));
    assert!(!stderr.contains("worker 'missing' closed"));
}

#[test]
fn returns_closed_backstop_when_the_directory_is_removed_mid_wait() {
    let workspace = temp_workspace("niles-wait-removed");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"working: still running\n");

    let waiter = spawn_wait(
        &workspace,
        &["auth-fix", "--interval", "0.05", "--timeout", "5"],
    );
    // The removal has to land after target resolution, or this is the unknown-id path instead.
    settle();
    remove_dir_all_eventually(&worker_dir);

    let output = waiter.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(10));
    assert_eq!(
        stdout_of(&output),
        "closed: worker 'auth-fix' directory removed\n"
    );
    let stderr = stderr_of(&output);
    assert!(stderr.contains("worker 'auth-fix' closed"));
    assert!(!stderr.contains("timeout"));
}

#[test]
fn a_symlinked_cursor_path_is_refused_rather_than_followed() {
    let workspace = temp_workspace("niles-wait-symlink");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"done: ready\n");
    let outside = workspace.join("outside.txt");
    fs::write(&outside, "untouched\n").unwrap();
    std::os::unix::fs::symlink(&outside, worker_dir.join("status.cursor")).unwrap();

    let output = run_wait(
        &workspace,
        &["auth-fix", "--interval", "0.05", "--timeout", "0"],
    );

    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(&outside).unwrap(),
        "untouched\n",
        "the cursor write followed a symlink out of the worker directory"
    );
}

#[test]
fn rejects_both_worker_and_task_selectors() {
    let workspace = temp_workspace("niles-wait-selectors");

    let output = run_wait(
        &workspace,
        &["auth-fix", "--task", "auth", "--timeout", "0"],
    );

    assert!(!output.status.success());
    let combined = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert!(
        combined.contains("cannot be used with") || combined.contains("use either"),
        "output: {combined}"
    );
}

#[test]
fn caps_an_enormous_status_line_instead_of_flooding_the_manager() {
    let workspace = temp_workspace("niles-wait-huge");
    let mut status = b"done: ".to_vec();
    status.extend(std::iter::repeat_n(b'x', 64 * 1024));
    status.push(b'\n');
    worker_with_status(&workspace, "auth-fix", &status);

    let output = run_wait(
        &workspace,
        &["auth-fix", "--interval", "0.05", "--timeout", "0"],
    );

    assert_command_success("wait with an enormous line", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.starts_with("done: xxx"));
    assert!(stdout.contains("(truncated)"), "stdout was not capped");
    assert!(stdout.len() < 8 * 1024, "stdout was {} bytes", stdout.len());
}

#[test]
fn task_label_waits_on_every_live_worker_carrying_it() {
    let workspace = temp_workspace("niles-wait-task");
    write_task_worker(&workspace, "alpha", "auth", b"working: nothing yet\n");
    write_task_worker(&workspace, "beta", "auth", b"blocked: needs a decision\n");
    write_task_worker(&workspace, "gamma", "other", b"done: unrelated task\n");

    let output = run_wait(
        &workspace,
        &["--task", "auth", "--interval", "0.05", "--timeout", "0"],
    );

    assert_command_success("wait --task", &output);
    // gamma carries a different label, so its wake must not satisfy this wait.
    assert_eq!(stdout_of(&output), "beta: blocked: needs a decision\n");
}

/// Writes a worker directory complete enough for task-label selection to find it.
fn write_task_worker(workspace: &Path, id: &str, task_label: &str, status: &[u8]) {
    let worker_dir = worker_with_status(workspace, id, status);
    fs::write(
        worker_dir.join("meta.json"),
        format!(
            r#"{{
  "niles_schema": 2,
  "id": "{id}",
  "agent": "codex",
  "project": "{}",
  "window": "niles:niles-{id}",
  "brief": "{}",
  "launch": "{}",
  "task_label": "{task_label}"
}}
"#,
            workspace.display(),
            worker_dir.join("brief.md").display(),
            worker_dir.join("launch.sh").display(),
        ),
    )
    .unwrap();
}

#[test]
fn send_advances_the_cursor_so_a_pre_send_line_cannot_satisfy_the_wait_after_it() {
    let workspace = temp_workspace("niles-send-cursor");
    let server = TmuxServer::start(&workspace, "send-cursor");
    server.new_window("niles-auth-fix");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"working: starting\n");
    write_worker_meta(&workspace, &server.session, "auth-fix", None);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_stub_agent(&bin);

    // The worker reports done, and the operator sends a follow-up without waiting first.
    let mut status = fs::OpenOptions::new()
        .append(true)
        .open(worker_dir.join("status.log"))
        .unwrap();
    writeln!(status, "done: first pass").unwrap();

    let send = niles_in(
        &server,
        &workspace,
        &bin,
        &["send", "auth-fix", "another", "pass", "please"],
    )
    .output()
    .unwrap();
    assert_command_success("send", &send);
    // The wake it stepped over is surfaced, not dropped silently.
    assert!(
        stderr_of(&send).contains("skipped unconsumed wake: done: first pass"),
        "stderr: {}",
        stderr_of(&send)
    );

    // The pre-send `done:` must not satisfy the wait that follows the send.
    let waited = niles_in(
        &server,
        &workspace,
        &bin,
        &["wait", "auth-fix", "--interval", "0.05", "--timeout", "0"],
    )
    .output()
    .unwrap();
    assert_eq!(
        waited.status.code(),
        Some(22),
        "stdout: {}",
        stdout_of(&waited)
    );
}

#[test]
fn send_wait_blocks_for_the_reply_that_follows_the_message() {
    let workspace = temp_workspace("niles-send-wait");
    let server = TmuxServer::start(&workspace, "send-wait");
    server.new_window("niles-auth-fix");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"done: stale pass\n");
    write_worker_meta(&workspace, &server.session, "auth-fix", None);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_stub_agent(&bin);

    let child = niles_in(
        &server,
        &workspace,
        &bin,
        // `--wait` written after the id, which is where clap's trailing var-arg would
        // otherwise swallow it into the message and type it into the agent's pane.
        &["send", "auth-fix", "--wait", "keep", "going"],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
    settle();

    let mut status = fs::OpenOptions::new()
        .append(true)
        .open(worker_dir.join("status.log"))
        .unwrap();
    writeln!(status, "done: second pass").unwrap();

    let output = child.wait_with_output().unwrap();
    assert_command_success("send --wait", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains("sent: auth-fix"), "stdout: {stdout}");
    // The reply, not the `done:` that was already sitting in the log before the send.
    assert!(stdout.contains("done: second pass"), "stdout: {stdout}");
    assert!(!stdout.contains("stale pass"), "stdout: {stdout}");
}

/// A worker whose window is gone can never append again, so waiting out the timeout on it is
/// pure delay — with the default timeout, an hour of it.
#[test]
fn a_gone_window_ends_the_wait_instead_of_blocking() {
    let workspace = temp_workspace("niles-wait-window-gone");
    // The session is real and live; this worker's window was never created in it.
    let server = TmuxServer::start(&workspace, "window-gone");
    worker_with_status(&workspace, "auth-fix", b"working: still running\n");
    write_worker_meta(&workspace, &server.session, "auth-fix", None);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_stub_agent(&bin);
    assert!(!server.windows().contains("niles-auth-fix"));

    let started = Instant::now();
    let output = niles_in(
        &server,
        &workspace,
        &bin,
        &["wait", "auth-fix", "--interval", "0.05", "--timeout", "30"],
    )
    .output()
    .unwrap();

    assert!(
        started.elapsed() < Duration::from_secs(10),
        "wait did not return promptly: {}\n{}",
        stderr_of(&output),
        server.diagnostics()
    );
    assert_eq!(output.status.code(), Some(10), "{}", stderr_of(&output));
    assert!(
        stdout_of(&output).contains("exited without reporting"),
        "stdout: {}",
        stdout_of(&output)
    );
}

/// A worker can report and *then* exit. The report must win; losing it would turn a completed
/// task into a crash report.
#[test]
fn a_final_line_is_delivered_before_a_gone_window_is_reported() {
    let workspace = temp_workspace("niles-wait-window-gone-late-line");
    let server = TmuxServer::start(&workspace, "gone-late-line");
    worker_with_status(&workspace, "auth-fix", b"done: finished the work\n");
    write_worker_meta(&workspace, &server.session, "auth-fix", None);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_stub_agent(&bin);

    let args = ["wait", "auth-fix", "--interval", "0.05", "--timeout", "5"];
    let reported = niles_in(&server, &workspace, &bin, &args).output().unwrap();
    assert_command_success("wait with a final line", &reported);
    assert_eq!(stdout_of(&reported), "done: finished the work\n");

    // Only once the log is drained does the gone window become the answer.
    let drained = niles_in(&server, &workspace, &bin, &args).output().unwrap();
    assert_eq!(
        drained.status.code(),
        Some(10),
        "{}\n{}",
        stderr_of(&drained),
        server.diagnostics()
    );
    assert!(stdout_of(&drained).contains("exited without reporting"));
}

/// A live window must never be mistaken for a gone one.
#[test]
fn a_live_window_keeps_the_wait_running() {
    let workspace = temp_workspace("niles-wait-window-live");
    let server = TmuxServer::start(&workspace, "window-live");
    server.new_window("niles-auth-fix");
    worker_with_status(&workspace, "auth-fix", b"working: still running\n");
    write_worker_meta(&workspace, &server.session, "auth-fix", None);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_stub_agent(&bin);

    let output = niles_in(
        &server,
        &workspace,
        &bin,
        &["wait", "auth-fix", "--interval", "0.05", "--timeout", "1"],
    )
    .output()
    .unwrap();

    assert_eq!(output.status.code(), Some(22), "{}", stdout_of(&output));
    assert!(stderr_of(&output).contains("timeout"));
}

/// A single-worker turn should not need a bare `niles wait` after the spawn.
#[test]
fn spawn_wait_blocks_for_the_workers_first_report() {
    let workspace = temp_workspace("niles-spawn-wait");
    let server = TmuxServer::start(&workspace, "spawn-wait");
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_stub_agent(&bin);

    // `--wait` after the id, where clap's trailing var-arg would otherwise swallow it into the
    // task text and write it into the worker's brief.
    let child = niles_in(
        &server,
        &workspace,
        &bin,
        &["spawn", "w1", "--wait", "fix", "the", "login", "bug"],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();

    // Poll for `meta.json`, which spawn writes last — after the window exists and is tagged. The
    // brief lands earlier, so waiting on it says nothing about whether the window is up yet.
    wait_for_file(&workspace.join(".niles/worker/w1/meta.json"));
    let brief = fs::read_to_string(workspace.join(".niles/worker/w1/brief.md")).unwrap();
    assert!(brief.contains("fix the login bug"), "{brief}");
    assert!(!brief.contains("--wait"), "the flag leaked into the task: {brief}");
    assert!(server.windows().contains("niles-w1"), "spawn created no window");

    let mut status = fs::OpenOptions::new()
        .append(true)
        .open(workspace.join(".niles/worker/w1/status.log"))
        .unwrap();
    writeln!(status, "done: first report").unwrap();

    let output = child.wait_with_output().unwrap();
    assert_command_success("spawn --wait", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains("spawned: w1"), "{stdout}");
    assert!(stdout.contains("done: first report"), "{stdout}");
}

/// Without `--wait`, spawn returns as soon as the window is up.
#[test]
fn spawn_without_wait_returns_immediately() {
    let workspace = temp_workspace("niles-spawn-no-wait");
    let server = TmuxServer::start(&workspace, "spawn-no-wait");
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_stub_agent(&bin);

    let started = Instant::now();
    let output = niles_in(&server, &workspace, &bin, &["spawn", "w1", "do", "the", "thing"])
        .output()
        .unwrap();

    assert_command_success("spawn", &output);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "spawn without --wait should not block"
    );
    assert!(!stdout_of(&output).contains("done:"));
    assert!(server.windows().contains("niles-w1"));
}

/// A private tmux server on its own socket, torn down when the test ends.
///
/// These tests assert on tmux window state, and a stub that answers window queries is a second
/// implementation of tmux that has to stay correct as niles's use of it grows — the previous stub
/// claimed to create a window and then reported none existed, which hid a real bug. A per-test
/// socket keeps parallel tests from seeing each other's sessions, and keeps them out of the
/// developer's own tmux.
struct TmuxServer {
    socket: std::path::PathBuf,
    session: String,
}

impl TmuxServer {
    fn start(workspace: &Path, session: &str) -> Self {
        // Not under the workspace: a unix socket path is capped near 104 bytes on macOS, and the
        // temp workspace names are long enough on their own to blow it.
        //
        // Named from a counter, not a timestamp. `SystemTime` is microsecond-resolution here, so
        // two tests starting in the same microsecond got the same socket: the second joined the
        // first's server, and the first's `Drop` then killed it mid-test.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let socket =
            std::path::PathBuf::from(format!("/tmp/nt-{}-{unique}.sock", std::process::id()));
        let server = Self {
            socket,
            session: session.to_owned(),
        };
        // The session runs a long-lived command rather than a shell. A shell that exits takes
        // the last window with it, which destroys the session — and a worker whose whole tmux
        // server has gone is a different state from one whose window has, with different
        // answers from every query after it.
        server.run(&[
            "new-session",
            "-d",
            "-s",
            session,
            "-c",
            &workspace.display().to_string(),
            "sleep 600",
        ]);
        server
    }

    /// The value niles reads from `$TMUX` to find this server.
    fn tmux_env(&self) -> String {
        format!("{},0,0", self.socket.display())
    }

    fn new_window(&self, name: &str) {
        self.run(&["new-window", "-d", "-t", &self.session, "-n", name]);
    }

    /// Whatever tmux says about this server right now, for failure messages.
    fn diagnostics(&self) -> String {
        let sessions = Command::new("tmux")
            .args(["-S", &self.socket.display().to_string(), "list-sessions"])
            .output()
            .unwrap();
        format!(
            "socket={} exists={} sessions={:?}/{:?} windows={:?}",
            self.socket.display(),
            self.socket.exists(),
            String::from_utf8_lossy(&sessions.stdout),
            String::from_utf8_lossy(&sessions.stderr),
            self.windows()
        )
    }

    fn windows(&self) -> String {
        let output = Command::new("tmux")
            .args(["-S", &self.socket.display().to_string()])
            .args(["list-windows", "-t", &format!("={}", self.session), "-F", "#{window_name}"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn run(&self, args: &[&str]) {
        let output = Command::new("tmux")
            .args(["-S", &self.socket.display().to_string()])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "tmux {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

impl Drop for TmuxServer {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-S", &self.socket.display().to_string(), "kill-server"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = fs::remove_file(&self.socket);
    }
}

/// Runs a niles command against `server`, with a stub agent binary on PATH.
fn niles_in(server: &TmuxServer, workspace: &Path, bin: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_niles"));
    command
        .args(args)
        .current_dir(workspace)
        .env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()))
        .env("NILES_HOME", niles_home(workspace))
        .env("TMUX", server.tmux_env());
    command
}

/// Blocks until `path` exists, or fails the test saying it never appeared.
fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("{} never appeared", path.display());
}

fn write_stub_agent(bin: &Path) {
    let codex = bin.join("codex");
    fs::write(
        &codex,
        "#!/bin/sh\ncase \"$1\" in --version) echo 'codex-cli 0.144.1'; exit 0 ;; esac\nsleep 30\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&codex).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    fs::set_permissions(&codex, permissions).unwrap();
}

fn write_worker_meta(workspace: &Path, session: &str, id: &str, task_label: Option<&str>) {
    let worker_dir = workspace.join(".niles/worker").join(id);
    let label = task_label
        .map(|l| format!(",\n  \"task_label\": \"{l}\""))
        .unwrap_or_default();
    fs::write(
        worker_dir.join("meta.json"),
        format!(
            r#"{{
  "niles_schema": 2,
  "id": "{id}",
  "agent": "codex",
  "project": "{}",
  "window": "{session}:niles-{id}",
  "brief": "{}",
  "launch": "{}"{label}
}}
"#,
            workspace.display(),
            worker_dir.join("brief.md").display(),
            worker_dir.join("launch.sh").display(),
        ),
    )
    .unwrap();
}

fn remove_dir_all_eventually(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match fs::remove_dir_all(path) {
            Ok(()) => return,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
            Err(_err) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
                if !path.exists() {
                    return;
                }
            }
            Err(err) => panic!("failed to remove {}: {err}", path.display()),
        }
    }
}
