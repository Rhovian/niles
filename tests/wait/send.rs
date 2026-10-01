use super::support::*;

#[test]
fn send_advances_the_cursor_so_a_pre_send_line_cannot_satisfy_the_wait_after_it() {
    let lab = Lab::start("niles-send-cursor");
    let worker_dir = lab.worker_window("auth-fix", b"working: starting\n");

    // The worker reports done, and the operator sends a follow-up without waiting first.
    append_status(&worker_dir.join("status.log"), b"done: first pass\n");

    let send = lab
        .niles(&["send", "auth-fix", "another", "pass", "please"])
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
    let waited = lab
        .niles(&["wait", "auth-fix", "--interval", "0.05", "--timeout", "0"])
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
    let lab = Lab::start("niles-send-wait");
    let worker_dir = lab.worker_window("auth-fix", b"done: stale pass\n");

    let child = lab
        .niles(
            // `--wait` written after the id, which is where clap's trailing var-arg would
            // otherwise swallow it into the message and type it into the agent's pane.
            &["send", "auth-fix", "--wait", "keep", "going"],
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    settle();

    append_status(&worker_dir.join("status.log"), b"done: second pass\n");

    let output = child.wait_with_output().unwrap();
    assert_command_success("send --wait", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains("sent: auth-fix"), "stdout: {stdout}");
    // The reply, not the `done:` that was already sitting in the log before the send.
    assert!(stdout.contains("done: second pass"), "stdout: {stdout}");
    assert!(!stdout.contains("stale pass"), "stdout: {stdout}");
}

#[test]
fn send_exits_copy_mode_and_delivers_the_message_once() {
    let lab = Lab::start("niles-send-copy-mode");
    let delivered = lab.workspace.join("delivered");
    let ready = lab.workspace.join("ready");
    let complete = lab.workspace.join("complete");
    let command = format!(
        "touch {}; IFS= read -r line; printf '%s\\n' \"$line\" >> {}; touch {}; printf \
         'complete\\n'; exec sleep 600",
        ready.display(),
        delivered.display(),
        complete.display()
    );
    lab.server
        .new_window_running("niles-auth-fix", Some(&command));
    wait_for_file(&ready);
    lab.worker("auth-fix", None, b"");
    let target = format!("{}:niles-auth-fix", lab.server.session);

    lab.server
        .run(&["set-option", "-w", "-t", &target, "mode-keys", "vi"]);
    lab.server.run(&["copy-mode", "-t", &target]);
    assert_eq!(lab.server.pane_in_mode(&target), "1");

    let send = lab
        .niles(&["send", "auth-fix", "delivered"])
        .output()
        .unwrap();

    assert_command_success("send from copy mode", &send);
    wait_for_file(&complete);
    assert_eq!(fs::read_to_string(delivered).unwrap(), "delivered\n");
}

/// A submit that the pane swallows must fail the send, not be reported as `sent:`. The pane's
/// echo makes the message visible while tmux trims the submitted newline from its capture.
#[test]
fn send_fails_when_the_submit_key_leaves_the_pane_unchanged() {
    let lab = Lab::start("niles-send-swallowed");
    lab.server
        .new_window_running("niles-auth-fix", Some("cat > /dev/null"));
    lab.worker("auth-fix", None, b"");

    let send = lab
        .niles(&["send", "auth-fix", "first", "line", "of", "the", "message"])
        .output()
        .unwrap();

    assert!(
        !send.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&send),
        stderr_of(&send)
    );
    assert!(
        !stdout_of(&send).contains("sent:"),
        "a swallowed submit must not be reported as sent; stdout: {}",
        stdout_of(&send)
    );
    let stderr = stderr_of(&send);
    assert!(
        stderr.contains("submit key did not take"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("niles peek"), "stderr: {stderr}");
}
