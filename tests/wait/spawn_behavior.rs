use super::support::*;

/// A single-worker turn should not need a bare `niles wait` after the spawn.
#[test]
fn spawn_wait_blocks_for_the_workers_first_report() {
    let lab = Lab::start("niles-spawn-wait");

    // Dispatch flags remain options even when they follow every task word.
    let child = lab
        .niles(&[
            "spawn", "--role", "research", "w1", "--agent", "codex", "fix", "the", "login", "bug",
            "--wait",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Poll for `meta.json`, which spawn writes last — after the window exists and is tagged. The
    // brief lands earlier, so waiting on it says nothing about whether the window is up yet.
    wait_for_file(&lab.workspace.join(".niles/worker/w1/meta.json"));
    let brief = fs::read_to_string(lab.workspace.join(".niles/worker/w1/brief.md")).unwrap();
    assert!(brief.contains("fix the login bug"), "{brief}");
    assert!(
        !brief.contains("--wait"),
        "the flag leaked into the task: {brief}"
    );
    assert!(
        lab.server.windows().contains("niles-w1"),
        "spawn created no window"
    );

    append_status(
        &lab.workspace.join(".niles/worker/w1/status.log"),
        b"done: first report\n",
    );

    let output = child.wait_with_output().unwrap();
    assert_command_success("spawn --wait", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains("spawned: w1"), "{stdout}");
    assert!(stdout.contains("done: first report"), "{stdout}");
}
