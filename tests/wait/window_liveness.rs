use super::support::*;

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
    lab.worker("d", None, b"done: finished the work\n");
    let output = lab.wait_now();
    assert_command_success("gone batch", &output);
    let text = stdout_of(&output);
    assert_eq!(text.lines().count(), 4, "{text}");
    assert!(text.contains("c done: ready\n"));
    assert!(text.contains("d done: finished the work\n"));
    assert!(text.contains("a gone: worker window or agent exited without reporting"));
    assert!(text.contains("b gone: worker window or agent exited without reporting"));
    let output = lab.wait_now();
    assert_drained(&output);
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
