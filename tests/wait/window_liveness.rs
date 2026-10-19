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
