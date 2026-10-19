use super::support::*;

#[test]
fn send_advances_the_cursor_so_a_pre_send_line_cannot_satisfy_the_wait_after_it() {
    let lab = Lab::start("niles-send-cursor");
    let worker_dir = receiving_worker(&lab, b"working: starting\n");

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
        .niles(&["wait", "--interval", "50ms", "--timeout", "0"])
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
    let worker_dir = receiving_worker(&lab, b"done: stale pass\n");

    let child = lab
        .niles(
            // Dispatch flags remain options even when they follow every message word.
            &["send", "auth-fix", "keep", "going", "--wait"],
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The wait takes its claim only after `send` has moved the cursor past the stale line, so the
    // reply appended now cannot be skipped by that advance.
    wait_for_file(&state_dir(&lab.workspace).join("worker/wait.lock"));

    append_status(&worker_dir.join("status.log"), b"done: second pass\n");

    let output = child.wait_with_output().unwrap();
    assert_command_success("send --wait", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains("sent: auth-fix"), "stdout: {stdout}");
    // The reply, not the `done:` that was already sitting in the log before the send.
    assert!(
        stdout.contains("auth-fix done: second pass"),
        "stdout: {stdout}"
    );
    assert!(!stdout.contains("stale pass"), "stdout: {stdout}");
}

#[test]
fn send_exits_copy_mode_and_delivers_the_message_once() {
    let lab = Lab::start("niles-send-copy-mode");
    receiving_worker(&lab, b"");
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
    let message = fs::read_to_string(lab.workspace.join("delivered")).unwrap();
    assert!(message.starts_with("\x1b[200~delivered\n\n## Reporting this follow-up\n"));
    assert_eq!(message.matches("## Reporting this follow-up").count(), 1);
}

fn receiving_worker(lab: &Lab, status: &[u8]) -> PathBuf {
    let ready = lab.workspace.join("ready");
    // Literal newlines belong to the paste; only the submit key completes the message.
    // Leave paste mode unadvertised: framing must not depend on newer tmux format fields.
    let command = format!(
        "stty -icanon -icrnl; touch {}; perl -e '$/ = \"\\r\"; print scalar <STDIN>' > {}; \
         printf 'complete\\n'; exec sleep 600",
        ready.display(),
        lab.workspace.join("delivered").display()
    );
    lab.server
        .new_window_running("niles-auth-fix", Some(&command));
    wait_for_file(&ready);
    lab.worker("auth-fix", None, status)
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

#[test]
fn multiline_stdin_is_one_complete_bracketed_paste_for_every_family() {
    let lab = Lab::start("niles-send-multiline");
    let body = format!(
        "BEGIN\n{}\nEND",
        "multiline follow-up with Unicode: café 🦀\n".repeat(16)
    );
    for family in ["claude", "codex", "hermes", "pi"] {
        let dir = receiving_worker(&lab, b"");
        let meta_path = dir.join("meta.json");
        let meta = fs::read_to_string(&meta_path)
            .unwrap()
            .replace("\"agent\": \"codex\"", &format!("\"agent\": \"{family}\""));
        fs::write(meta_path, meta).unwrap();
        let mut child = lab
            .niles(&["send", "auth-fix", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        std::io::Write::write_all(&mut child.stdin.take().unwrap(), body.as_bytes()).unwrap();
        let sent = child.wait_with_output().unwrap();
        assert_command_success(family, &sent);
        let dir = fs::canonicalize(dir).unwrap();
        let report = dir.join("report.md");
        let status = dir.join("status.log");
        let footer = include_str!("../../prompts/followup_reporting.md")
            .replace("{report_path}", report.to_str().unwrap())
            .replace(
                "{status_command}",
                &format!(
                    "printf '%s\\n' 'done: <short result>; report: {}' >> '{}'",
                    report.display(),
                    status.display()
                ),
            );
        assert_eq!(
            fs::read_to_string(lab.workspace.join("delivered")).unwrap(),
            format!("\x1b[200~{body}\n\n{footer}\x1b[201~\r"),
            "{family}"
        );
        lab.server
            .run(&["kill-window", "-t", "niles:niles-auth-fix"]);
        fs::remove_file(lab.workspace.join("ready")).unwrap();
    }
}

#[test]
fn send_rejects_an_embedded_paste_terminator() {
    let lab = Lab::start("niles-send-escape");
    receiving_worker(&lab, b"");
    let sent = lab
        .niles(&["send", "auth-fix", "prefix\x1b[201~suffix"])
        .output()
        .unwrap();
    assert!(!sent.status.success());
    assert!(!stdout_of(&sent).contains("sent:"));
    assert!(stderr_of(&sent).contains("escape character"));
    assert!(
        fs::read(lab.workspace.join("delivered"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn send_wait_takes_over_the_workspace_and_collects_other_workers_reports() {
    let lab = Lab::start("niles-send-workspace");
    let dir = receiving_worker(&lab, b"working: ready\n");
    let old = lab
        .niles(&["wait", "--interval", "1h", "--timeout", "1h"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_file(&dir.join("status.cursor"));
    lab.worker("other", None, b"done: unrelated report\n");
    let output = lab
        .niles(&["send", "auth-fix", "follow up", "--wait"])
        .output()
        .unwrap();
    assert_command_success("send workspace wait", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains("sent: auth-fix"), "{stdout}");
    assert!(
        stdout.contains("other done: unrelated report\n"),
        "{stdout}"
    );
    let output = old.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(11));
    assert!(stdout_of(&output).is_empty());
}

#[test]
fn a_silent_followup_with_checkins_off_still_produces_one_gone_wake() {
    let lab = Lab::start("niles-send-gone-off");
    let dir = receiving_worker(&lab, b"done: old\n");
    let output = lab
        .niles(&["send", "auth-fix", "follow up", "--checkin", "off"])
        .output()
        .unwrap();
    assert_command_success("send without checkin", &output);
    assert!(!dir.join("checkin").exists());
    lab.server
        .run(&["kill-window", "-t", "niles:niles-auth-fix"]);
    let output = lab.niles(&["wait", "--timeout", "0"]).output().unwrap();
    assert_command_success("silent followup gone", &output);
    assert!(stdout_of(&output).starts_with("auth-fix gone:"));
    let output = lab.niles(&["wait", "--timeout", "0"]).output().unwrap();
    assert_eq!(output.status.code(), Some(22));
    assert!(stdout_of(&output).is_empty());
}
