use super::support::*;
use std::{io::Write, process::Stdio};

#[test]
fn auth_spawn_peek_and_send_use_tmux_worker_metadata() {
    let env = TestEnv::new("niles-worker-test");
    let pane_file = env.root.join("pane.txt");

    let spawn = env
        .niles(
            &env.root,
            &[
                "spawn", "--role", "research", "auth-fix", "--task", "auth", "--agent", "claude",
                "Fix", "auth",
            ],
        )
        .env("TMUX_PANE_FILE", &pane_file)
        .output()
        .unwrap();
    assert_command_success("spawn", &spawn);
    let spawn_stdout = String::from_utf8_lossy(&spawn.stdout);
    assert!(spawn_stdout.contains("spawned: auth-fix"));
    assert!(spawn_stdout.contains("window: niles-auth-fix"));
    assert!(spawn_stdout.contains("task: auth"));
    assert!(spawn_stdout.contains("peek: niles peek auth-fix"));
    assert!(spawn_stdout.contains("report: niles report auth-fix"));
    assert!(spawn_stdout.contains("close: niles close auth-fix"));
    assert!(spawn_stdout.contains("close_task: niles close --task auth"));
    assert!(spawn_stdout.contains("workers: niles workers"));

    let meta = fs::read_to_string(env.root.join(".niles/worker/auth-fix/meta.json")).unwrap();
    assert!(meta.contains("\"agent\": \"claude\""));
    assert!(meta.contains("\"task_label\": \"auth\""));
    assert!(meta.contains("\"created_at\":"));
    assert!(meta.contains("\"window\": \"niles-test-session:niles-auth-fix\""));
    let target = "=niles-test-session:=niles-auth-fix";
    let project = env.root.display();

    let brief = fs::read_to_string(env.root.join(".niles/worker/auth-fix/brief.md")).unwrap();
    assert!(brief.contains("task_label: auth"));
    assert!(brief.contains("Fix auth"));
    assert!(brief.contains("report_file:"));
    assert!(brief.contains(".niles/worker/auth-fix/report.md"));
    assert!(brief.contains("done: <short result>; report:"));
    assert!(brief.contains("You are the research"));
    assert!(brief.contains("Do not run the gate"));
    assert!(!brief.contains("You are the reviewer"));
    assert!(!brief.contains("name the attacker"));

    let launch = fs::read_to_string(env.root.join(".niles/worker/auth-fix/launch.sh")).unwrap();
    assert!(launch.contains("CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=false"));
    assert!(launch.contains("CLAUDE_CODE_DISABLE_AGENT_VIEW=1"));
    // The agent runs as a child, not via exec, so the script survives to report its exit.
    assert!(launch.contains("'claude'"), "{launch}");
    assert!(!launch.contains("exec "), "{launch}");
    assert!(launch.contains("|| code=$?"), "{launch}");
    assert!(launch.contains(">> \"$STATUS\""), "{launch}");

    let peek = env
        .niles(&env.root, &["peek", "auth-fix", "--lines", "7"])
        .env("TMUX_PANE_FILE", &pane_file)
        .output()
        .unwrap();
    assert!(peek.status.success());
    assert_eq!(String::from_utf8_lossy(&peek.stdout), "pane output\n");

    let send = env
        .niles(&env.root, &["send", "auth-fix", "continue", "please"])
        .env("TMUX_PANE_FILE", &pane_file)
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

    let log = env.tmux_log();
    assert!(log.contains("display-message -p #S"));
    assert!(
        !log.contains("new-session"),
        "niles must not create tmux sessions; it uses the one it was run from"
    );
    assert!(log.contains("new-window -d -t =niles-test-session: -n niles-auth-fix"));
    assert!(log.contains(": -n niles-auth-fix"));
    assert!(
        log.contains(&format!(
            "set-option -w -t {target} @niles-project {project}"
        )),
        "{log}"
    );
    assert!(log.contains(&format!(
        "set-option -w -t {target} @niles-worker-id auth-fix"
    )));
    assert!(log.contains(&format!("capture-pane -p -t {target} -S -7")));
    assert!(log.contains(&format!("send-keys -t {target} -l continue please")));
    assert!(log.contains(&format!("send-keys -t {target} C-m")));
}

#[test]
fn send_accepts_stdin_and_literal_flag_text() {
    let env = TestEnv::new("niles-worker-send-message-input");
    let pane_file = env.root.join("pane.txt");
    let spawn = env.run(&[
        "spawn", "--role", "research", "worker", "--agent", "claude", "task",
    ]);
    assert_command_success("fixture spawn", &spawn);

    let mut child = env
        .niles(&env.root, &["send", "worker", "-"])
        .env("TMUX_PANE_FILE", &pane_file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"  stdin first\nstdin second\n\n")
        .unwrap();
    let stdin = child.wait_with_output().unwrap();
    assert_command_success("send stdin", &stdin);
    assert_eq!(
        fs::read_to_string(&pane_file).unwrap(),
        "composer:   stdin first\nstdin second\n\nsubmitted\n"
    );

    let literal = env
        .niles(&env.root, &["send", "worker", "--", "--wait"])
        .env("TMUX_PANE_FILE", &pane_file)
        .output()
        .unwrap();
    assert_command_success("send literal --wait", &literal);
    assert!(stdout_of(&literal).contains("wait: niles wait worker"));

    let log = env.tmux_log();
    for message in ["  stdin first\nstdin second\n", "--wait"] {
        assert!(
            log.contains(&format!(
                "send-keys -t =niles-test-session:=niles-worker -l {message}"
            )),
            "message {message:?} was not delivered verbatim:\n{log}"
        );
    }
}
