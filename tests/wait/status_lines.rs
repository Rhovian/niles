use super::support::*;

#[test]
fn does_not_redeliver_consumed_wake_and_delivers_next() {
    let (workspace, worker_dir, first) = wait_once(b"done: first\n");
    let args = ["--interval", "50ms", "--timeout", "0"];
    assert_command_success("first wait", &first);
    assert_eq!(stdout_of(&first), "auth-fix done: first\n");
    assert_eq!(cursor(&worker_dir), "12\n");

    let second = run_wait(&workspace, &args);
    assert!(!second.status.success());
    assert_drained(&second);
    assert!(stderr_of(&second).contains("timeout"));

    append_status(&worker_dir.join("status.log"), b"done: second\n");

    let third = run_wait(&workspace, &args);
    assert_command_success("third wait", &third);
    assert_eq!(stdout_of(&third), "auth-fix done: second\n");
    assert_eq!(cursor(&worker_dir), "25\n");
    // Rewrite the log shorter than the recorded offset of 25.
    fs::write(worker_dir.join("status.log"), b"done: new\n").unwrap();

    let output = run_wait(&workspace, &args);
    assert_command_success("wait after truncation", &output);
    assert_eq!(stdout_of(&output), "auth-fix done: new\n");
    assert_eq!(cursor(&worker_dir), "10\n");
}

#[test]
fn skips_non_actionable_lines_without_persisting_past_an_undelivered_wake() {
    let (_, worker_dir, output) =
        wait_once(b"working: one\nworking: two\nneeds-decision: pick a lane\nworking: three\n");

    assert_command_success("wait past working lines", &output);
    assert_eq!(stdout_of(&output), "auth-fix needs-decision: pick a lane\n");
    // Stops just past the decision line, not at EOF: the trailing `working:` stays unscanned.
    assert_eq!(cursor(&worker_dir), "54\n");
}

#[test]
fn leaves_an_unterminated_trailing_line_for_the_next_poll() {
    let (workspace, worker_dir, partial) = wait_once(b"done: comp");
    assert_eq!(partial.status.code(), Some(22));
    // The cursor file exists because it doubles as the lock, but holds no advanced position.
    let recorded = cursor(&worker_dir);
    assert!(
        recorded.trim().is_empty() || recorded.trim() == "0",
        "a half-written line must not advance the cursor, found {recorded:?}"
    );

    append_status(&worker_dir.join("status.log"), b"lete\n");

    let whole = run_wait(&workspace, &["--interval", "50ms", "--timeout", "0"]);
    assert_command_success("wait after line completed", &whole);
    assert_eq!(stdout_of(&whole), "auth-fix done: complete\n");
}

#[test]
fn escapes_control_characters_instead_of_emitting_them() {
    let (_, _, output) = wait_once(b"done: shipped\x1b[2J\x1b[H\x07 and cleared your screen\n");

    assert_command_success("wait with control characters", &output);
    let stdout = stdout_of(&output);
    assert!(
        !stdout.contains('\x1b'),
        "raw escape reached stdout: {stdout:?}"
    );
    assert!(
        !stdout.contains('\x07'),
        "raw bell reached stdout: {stdout:?}"
    );
    assert!(stdout.contains("shipped"));
    assert!(stdout.contains("and cleared your screen"));
}

#[test]
fn non_utf8_bytes_do_not_desynchronise_the_cursor() {
    let mut status = b"working: \xff\xfe garbage\n".to_vec();
    status.extend_from_slice(b"done: recovered\n");
    let (_, worker_dir, output) = wait_once(&status);
    let total = fs::metadata(worker_dir.join("status.log")).unwrap().len();

    assert_command_success("wait past non-utf8", &output);
    assert_eq!(stdout_of(&output), "auth-fix done: recovered\n");
    // Offsets come from raw bytes, so lossy decoding of the garbage line cannot shift them.
    assert_eq!(cursor(&worker_dir), format!("{total}\n"));
}

#[test]
fn stage_does_not_wake_wait() {
    let workspace = temp_workspace("niles-wait-stage");
    worker_with_status(&workspace, "staged", b"stage: ready; gates: none\n");
    let output = run_wait(&workspace, &["--timeout", "0"]);
    assert_drained(&output);
}
