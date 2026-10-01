use super::support::*;

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
