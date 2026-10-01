use super::support::*;

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
        &[
            "spawn", "w1", "--wait", "--agent", "codex", "fix", "the", "login", "bug",
        ],
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
    assert!(
        !brief.contains("--wait"),
        "the flag leaked into the task: {brief}"
    );
    assert!(
        server.windows().contains("niles-w1"),
        "spawn created no window"
    );

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
    let output = niles_in(
        &server,
        &workspace,
        &bin,
        &["spawn", "w1", "--agent", "codex", "do", "the", "thing"],
    )
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
