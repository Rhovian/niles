use super::support::*;

#[test]
fn auth_spawn_peek_and_send_use_tmux_worker_metadata() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-test");
    let home = niles_home(&workspace);

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    let tmux = bin.join("tmux");
    // The pane is modelled rather than answered with a constant: `send` watches the pane to
    // decide whether the submit key took, so a stub whose capture never moves is a stub that
    // cannot tell a delivered message from a swallowed one.
    fs::write(
        &tmux,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  display-message) printf 'niles-test-session\n'; exit 0 ;;
  has-session) exit 1 ;;
  list-windows)
    if [ "$2" = "-a" ]; then
      exit 0
    fi
    if [ -n "${TMUX_WINDOWS:-}" ]; then
      printf '%s\n' "$TMUX_WINDOWS"
    fi
    exit 0
    ;;
  send-keys)
    if [ "$4" = "-l" ]; then
      printf 'composer: %s\n' "$5" >> "$TMUX_PANE_FILE"
    else
      printf 'submitted\n' >> "$TMUX_PANE_FILE"
    fi
    exit 0
    ;;
  capture-pane)
    printf 'pane output\n'
    if [ -f "$TMUX_PANE_FILE" ]; then
      cat "$TMUX_PANE_FILE"
    fi
    exit 0
    ;;
  *) exit 0 ;;
esac
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&tmux).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&tmux, permissions).unwrap();
    write_executable(
        &bin.join("claude"),
        r#"#!/bin/sh
exit 0
"#,
    );

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("PATH must be set in the test environment")
    );
    let pane_file = workspace.join("pane.txt");

    let spawn = Command::new(niles)
        .args([
            "spawn", "auth-fix", "--task", "auth", "--agent", "claude", "Fix", "auth",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_PANE_FILE", &pane_file)
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();
    assert!(
        spawn.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&spawn.stdout),
        String::from_utf8_lossy(&spawn.stderr)
    );
    let spawn_stdout = String::from_utf8_lossy(&spawn.stdout);
    assert!(spawn_stdout.contains("spawned: auth-fix"));
    assert!(spawn_stdout.contains("window: niles-auth-fix"));
    assert!(spawn_stdout.contains("task: auth"));
    assert!(spawn_stdout.contains("peek: niles peek auth-fix"));
    assert!(spawn_stdout.contains("report: niles report auth-fix"));
    assert!(spawn_stdout.contains("close: niles close auth-fix"));
    assert!(spawn_stdout.contains("close_task: niles close --task auth"));
    assert!(spawn_stdout.contains("workers: niles workers"));

    let meta = fs::read_to_string(workspace.join(".niles/worker/auth-fix/meta.json")).unwrap();
    assert!(meta.contains("\"agent\": \"claude\""));
    assert!(meta.contains("\"task_label\": \"auth\""));
    assert!(meta.contains("\"created_at\":"));
    let meta_json: serde_json::Value = serde_json::from_str(&meta).unwrap();
    let window = meta_json["window"].as_str().unwrap();
    let target = exact_target(window);
    let project = meta_json["project"].as_str().unwrap();
    assert_eq!(window, "niles-test-session:niles-auth-fix");

    let brief = fs::read_to_string(workspace.join(".niles/worker/auth-fix/brief.md")).unwrap();
    assert!(brief.contains("task_label: auth"));
    assert!(brief.contains("Fix auth"));
    assert!(brief.contains("report_file:"));
    assert!(brief.contains(".niles/worker/auth-fix/report.md"));
    assert!(brief.contains("done: <short result>; report:"));
    // Default role, so the worker fragment and none of the reviewer's doctrine.
    assert!(brief.contains("You are the worker"));
    assert!(brief.contains("You own the gate"));
    assert!(!brief.contains("You are the reviewer"));
    assert!(!brief.contains("name the attacker"));

    let launch = fs::read_to_string(workspace.join(".niles/worker/auth-fix/launch.sh")).unwrap();
    assert!(launch.contains("CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=false"));
    // The agent runs as a child, not via exec, so the script survives to report its exit.
    assert!(launch.contains("'claude'"), "{launch}");
    assert!(!launch.contains("exec "), "{launch}");
    assert!(launch.contains("|| code=$?"), "{launch}");
    assert!(launch.contains(">> \"$STATUS\""), "{launch}");

    let peek = Command::new(niles)
        .args(["peek", "auth-fix", "--lines", "7"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_PANE_FILE", &pane_file)
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();
    assert!(peek.status.success());
    assert_eq!(String::from_utf8_lossy(&peek.stdout), "pane output\n");

    let send = Command::new(niles)
        .args(["send", "auth-fix", "continue", "please"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_PANE_FILE", &pane_file)
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();
    assert!(send.status.success());
    let send_stdout = String::from_utf8_lossy(&send.stdout);
    assert!(send_stdout.contains("sent: auth-fix"));
    // `wait` first, as spawn prints it: the send just armed a wake, and collecting it is the
    // next move.
    assert!(
        send_stdout.contains("wait: niles wait auth-fix"),
        "{send_stdout}"
    );
    assert!(
        send_stdout.contains("peek: niles peek auth-fix"),
        "{send_stdout}"
    );

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(
        !log.contains("new-session"),
        "niles must not create tmux sessions; it uses the one it was run from"
    );
    assert!(log.contains("new-window -d -t =niles-test-session: -n niles-auth-fix"));
    assert!(log.contains(": -n niles-auth-fix"));
    assert!(log.contains(&format!(
        "set-option -w -t {target} @niles-project {project}"
    )));
    assert!(log.contains(&format!(
        "set-option -w -t {target} @niles-worker-id auth-fix"
    )));
    assert!(log.contains(&format!("capture-pane -p -t {target} -S -7")));
    assert!(log.contains(&format!("send-keys -t {target} -l continue please")));
    assert!(log.contains(&format!("send-keys -t {target} C-m")));

    // The pane is observed on both sides of the submit: settled after the paste, then checked for
    // the change that proves the submit took. Timing the gap instead is what let a swallowed
    // `C-m` be reported as `sent:`.
    let calls = log.lines().collect::<Vec<_>>();
    let paste = calls
        .iter()
        .position(|call| *call == format!("send-keys -t {target} -l continue please"))
        .expect("the message paste");
    let submit = calls
        .iter()
        .position(|call| *call == format!("send-keys -t {target} C-m"))
        .expect("the submit key");
    assert!(
        calls[paste..submit]
            .iter()
            .any(|call| call.starts_with("capture-pane")),
        "the pane must be watched between the paste and the submit:\n{log}"
    );
    assert!(
        calls[submit..]
            .iter()
            .any(|call| call.starts_with("capture-pane")),
        "the pane must be checked after the submit:\n{log}"
    );
}
