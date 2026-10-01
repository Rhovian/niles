use super::support::*;

/// A worker can report and *then* exit. The report must win; losing it would turn a completed
/// task into a crash report.
#[test]
fn a_final_line_is_delivered_before_a_gone_window_is_reported() {
    let lab = Lab::start("niles-wait-window-gone-late-line");
    lab.worker("auth-fix", None, b"done: finished the work\n");

    let args = ["wait", "auth-fix", "--interval", "0.05", "--timeout", "5"];
    let reported = lab.niles(&args).output().unwrap();
    assert_command_success("wait with a final line", &reported);
    assert_eq!(stdout_of(&reported), "done: finished the work\n");

    // Only once the log is drained does the gone window become the answer.
    let drained = lab.niles(&args).output().unwrap();
    assert_eq!(
        drained.status.code(),
        Some(10),
        "{}\n{}",
        stderr_of(&drained),
        lab.server.diagnostics()
    );
    assert!(stdout_of(&drained).contains("exited without reporting"));
}

/// A live window must never be mistaken for a gone one.
#[test]
fn a_live_window_keeps_the_wait_running() {
    let lab = Lab::start("niles-wait-window-live");
    lab.worker_window("auth-fix", b"working: still running\n");

    let output = lab
        .niles(&["wait", "auth-fix", "--interval", "0.05", "--timeout", "1"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(22), "{}", stdout_of(&output));
    assert!(stderr_of(&output).contains("timeout"));
}
