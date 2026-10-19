use super::support::*;

/// A worker can report and *then* exit. The report must win; losing it would turn a completed
/// task into a crash report.
#[test]
fn a_final_line_is_delivered_and_a_gone_window_does_not_end_the_next_wait() {
    let lab = Lab::start("niles-wait-window-gone-late-line");
    lab.worker("auth-fix", None, b"done: finished the work\n");

    let args = ["wait", "--interval", "50ms", "--timeout", "5s"];
    let reported = lab.niles(&args).output().unwrap();
    assert_command_success("wait with a final line", &reported);
    assert_eq!(stdout_of(&reported), "auth-fix done: finished the work\n");

    let drained = lab.niles(&["wait", "--timeout", "0"]).output().unwrap();
    assert_eq!(drained.status.code(), Some(22));
    assert!(stdout_of(&drained).is_empty());
}

/// A live window must never be mistaken for a gone one.
#[test]
fn a_live_window_keeps_the_wait_running() {
    let lab = Lab::start("niles-wait-window-live");
    lab.worker_window("auth-fix", b"working: still running\n");

    let output = lab
        .niles(&["wait", "--interval", "50ms", "--timeout", "1s"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(22), "{}", stdout_of(&output));
    assert!(stderr_of(&output).contains("timeout"));
}

#[test]
fn dead_unreported_workers_share_one_batch_and_are_not_replayed() {
    let lab = Lab::start("niles-wait-gone-batch");
    lab.worker(
        "a",
        None,
        b"working: busy\nclosed: agent exited (status 1)\n",
    );
    lab.worker("b", None, b"closed: agent exited (status 1)\n");
    lab.worker("c", None, b"done: ready\nclosed: agent exited (status 0)\n");
    let output = lab.niles(&["wait", "--timeout", "0"]).output().unwrap();
    assert_command_success("gone batch", &output);
    let text = stdout_of(&output);
    assert_eq!(text.lines().count(), 3, "{text}");
    assert!(text.contains("c done: ready\n"));
    assert!(text.contains("a gone: worker window or agent exited without reporting"));
    assert!(text.contains("b gone: worker window or agent exited without reporting"));
    let output = lab.niles(&["wait", "--timeout", "0"]).output().unwrap();
    assert_eq!(output.status.code(), Some(22));
    assert!(stdout_of(&output).is_empty());
}

#[test]
fn a_window_that_dies_during_the_wait_produces_a_gone_wake() {
    let lab = Lab::start("niles-wait-window-died");
    let dir = lab.worker_window("a", b"working: busy\n");
    let child = lab
        .niles(&["wait", "--interval", "20ms", "--timeout", "10s"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_file(&dir.join("status.cursor"));
    lab.server.run(&["kill-window", "-t", "niles:niles-a"]);
    let output = child.wait_with_output().unwrap();
    assert_command_success("window died during wait", &output);
    assert!(stdout_of(&output).starts_with("a gone:"));
}
