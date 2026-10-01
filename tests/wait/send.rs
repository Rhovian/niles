use super::support::*;

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

#[test]
fn send_exits_copy_mode_and_delivers_the_message_once() {
    let workspace = temp_workspace("niles-send-copy-mode");
    let server = TmuxServer::start(&workspace, "send-copy-mode");
    let delivered = workspace.join("delivered");
    let ready = workspace.join("ready");
    let complete = workspace.join("complete");
    let command = format!(
        "touch {}; IFS= read -r line; printf '%s\\n' \"$line\" >> {}; touch {}; printf \
         'complete\\n'; exec sleep 600",
        ready.display(),
        delivered.display(),
        complete.display()
    );
    server.new_window_running("niles-auth-fix", Some(&command));
    wait_for_file(&ready);
    worker_with_status(&workspace, "auth-fix", b"");
    write_worker_meta(&workspace, &server.session, "auth-fix", None);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_stub_agent(&bin);
    let target = format!("{}:niles-auth-fix", server.session);

    server.run(&["set-option", "-w", "-t", &target, "mode-keys", "vi"]);
    server.run(&["copy-mode", "-t", &target]);
    assert_eq!(server.pane_in_mode(&target), "1");

    let send = niles_in(
        &server,
        &workspace,
        &bin,
        &["send", "auth-fix", "delivered"],
    )
    .output()
    .unwrap();

    assert_command_success("send from copy mode", &send);
    wait_for_file(&complete);
    assert_eq!(fs::read_to_string(delivered).unwrap(), "delivered\n");
}

/// A submit that the pane swallows must fail the send, not be reported as `sent:`.
///
/// The pane runs `cat > /dev/null`: the terminal echoes the pasted text, and `C-m` adds a newline
/// that tmux then trims back off the capture — so the message is visibly staged and the submit
/// leaves the pane exactly as it was. That is the shape of the observed failure, where a TUI
/// still ingesting a few KB of paste swallowed the submit and the message sat unsent while niles
/// printed success.
#[test]
fn send_fails_when_the_submit_key_leaves_the_pane_unchanged() {
    let workspace = temp_workspace("niles-send-swallowed");
    let server = TmuxServer::start(&workspace, "send-swallowed");
    server.new_window_running("niles-auth-fix", Some("cat > /dev/null"));
    worker_with_status(&workspace, "auth-fix", b"");
    write_worker_meta(&workspace, &server.session, "auth-fix", None);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_stub_agent(&bin);

    let send = niles_in(
        &server,
        &workspace,
        &bin,
        &["send", "auth-fix", "first", "line", "of", "the", "message"],
    )
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

#[test]
fn send_does_not_submit_when_the_message_never_renders() {
    let workspace = temp_workspace("niles-send-no-render");
    let server = TmuxServer::start(&workspace, "send-no-render");
    let received = workspace.join("received");
    let ready = workspace.join("ready");
    let command = format!(
        "stty -echo; touch {}; cat > {}",
        ready.display(),
        received.display()
    );
    server.new_window_running("niles-auth-fix", Some(&command));
    wait_for_file(&ready);
    worker_with_status(&workspace, "auth-fix", b"");
    write_worker_meta(&workspace, &server.session, "auth-fix", None);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_stub_agent(&bin);

    let send = niles_in(
        &server,
        &workspace,
        &bin,
        &["send", "auth-fix", "message-without-submit"],
    )
    .output()
    .unwrap();

    assert!(!send.status.success(), "stdout: {}", stdout_of(&send));
    assert!(!stdout_of(&send).contains("sent:"));
    let stderr = stderr_of(&send);
    assert!(stderr.contains("message text never appeared"), "{stderr}");
    // With canonical terminal input, `cat` receives the staged line only after Enter. An empty
    // file therefore proves the failure path did not send the submit key.
    assert_eq!(fs::read_to_string(received).unwrap(), "");
}
