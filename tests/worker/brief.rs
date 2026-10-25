use super::support::*;
use std::{io::Write, process::Stdio};

#[test]
fn worker_spawn_records_mechanical_decisions_and_design_headers() {
    let env = TestEnv::new("niles-worker-design-record");
    let session = state_dir(&env.root).join("sessions/lead");
    fs::create_dir_all(&session).unwrap();
    let meta = serde_json::json!({
        "id": "lead", "agent": "claude", "created_at": "2026-01-02T03:04:05Z",
        "workspace": env.root, "brief": session.join("lead.md")
    });
    fs::write(
        session.join("session.json"),
        serde_json::to_vec(&meta).unwrap(),
    )
    .unwrap();
    let log = session.join("decisions.log");
    let mechanical = [
        "spawn",
        "mechanical",
        "--role",
        "worker",
        "--agent",
        "claude",
        "--mechanical",
        "rename",
        "task",
    ];
    fs::create_dir(&log).unwrap();
    assert_failure_contains(
        "unwritable decisions log",
        &env.run(&mechanical),
        "failed to append",
    );
    assert!(!state_dir(&env.root).join("worker/mechanical").exists());
    fs::remove_dir(&log).unwrap();
    fs::write(&log, "earlier decision").unwrap();
    let spawned = env.run(&mechanical);
    assert_command_success("mechanical worker", &spawned);
    let logged = fs::read_to_string(&log).unwrap();
    let line = logged.strip_prefix("earlier decision\n").unwrap();
    let (timestamp, decision) = line.split_once(' ').unwrap();
    chrono::DateTime::parse_from_rfc3339(timestamp).unwrap();
    assert_eq!(decision, "mechanical mechanical: rename\n");
    let workers = state_dir(&env.root).join("worker");
    let body = fs::read_to_string(workers.join("mechanical/brief.md")).unwrap();
    let (header, _) = body.split_once("\n\n## Task").unwrap();
    assert!(!header.contains("design_record:"));

    let designer = env.run(&[
        "spawn", "designer", "--role", "design", "--agent", "claude", "design",
    ]);
    assert_command_success("designer", &designer);
    fs::write(workers.join("designer/status.log"), "done: agreed record\n").unwrap();
    let spawned = env.run(&[
        "spawn",
        "implementation",
        "--role",
        "worker",
        "--agent",
        "claude",
        "--design",
        "designer",
        "task",
    ]);
    assert_command_success("worker with design", &spawned);
    let body = fs::read_to_string(workers.join("implementation/brief.md")).unwrap();
    assert!(body.contains(&format!(
        "report_file: {}\ndesign_record: {}\n",
        workers.join("implementation/report.md").display(),
        workers.join("designer/report.md").display()
    )));
}

/// Role behavior travels in the system prompt while the task remains in the opening turn.
#[test]
fn role_selects_primary_instructions_outside_the_opening_turn() {
    let env = TestEnv::new("niles-worker-roles");

    let mut briefs = Vec::new();
    for role in ["design", "reviewer", "security", "research", "worker"] {
        let spawn = env
            .niles(
                &env.root,
                &[
                    "spawn", role, "--role", role, "--agent", "claude", "Do", "it",
                ],
            )
            .output()
            .unwrap();
        assert_command_success(&format!("spawn --role {role}"), &spawn);

        let brief = fs::read_to_string(
            state_dir(&env.root)
                .join("worker")
                .join(role)
                .join("brief.md"),
        )
        .unwrap();
        assert!(!brief.contains(&format!("You are the {role}")), "{brief}");
        let launch = state_dir(&env.root).join(format!("worker/{role}/launch.sh"));
        let script = fs::read_to_string(launch).unwrap();
        assert!(script.contains("'--append-system-prompt'"), "{script}");
        assert!(script.contains(&format!("You are the {role}")), "{script}");
        assert!(brief.contains("niles agent status --help"), "{brief}");
        assert!(script.contains("Work from this brief and the code."));
        assert!(
            script.contains(
                "Deliverables go in the report file; the lead reads it, not your terminal."
            )
        );
        assert!(script.contains("Report uncertainty as uncertainty."));
        if role == "worker" {
            let (header, _) = brief.split_once("\n\n## Task").unwrap();
            assert!(!header.contains("design_record:"));
            assert!(script.contains("Without a design record, implement the issue directly"));
            assert!(!state_dir(&env.root).join("sessions").exists());
        }
        briefs.push(brief);
    }

    for (left, right) in [(0, 1), (0, 2), (1, 2)] {
        assert_ne!(
            briefs[left], briefs[right],
            "each role must get a different brief"
        );
    }
}

#[test]
fn stdin_task_preserves_multiline_text() {
    let env = TestEnv::new("niles-worker-stdin-task");
    let mut child = env
        .niles(
            &env.root,
            &[
                "spawn",
                "review-file",
                "--role",
                "reviewer",
                "--agent",
                "claude",
                "-",
            ],
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"  Inspect the parser edge cases.\nKeep this line verbatim.\n\n")
        .unwrap();
    let spawn = child.wait_with_output().unwrap();
    assert_command_success("spawn stdin task", &spawn);

    let brief =
        fs::read_to_string(state_dir(&env.root).join("worker/review-file/brief.md")).unwrap();
    assert!(brief.contains("niles agent status --help"), "{brief}");
    let (_, task) = brief.split_once("## Task\n\n").unwrap();
    assert_eq!(
        task.trim_end_matches('\n'),
        "  Inspect the parser edge cases.\nKeep this line verbatim."
    );
}

#[test]
fn spawn_accepts_literal_flag_text() {
    let env = TestEnv::new("niles-worker-spawn-message-options");

    let literal = env.run(&[
        "spawn", "literal", "--role", "research", "--agent", "claude", "--", "--wait",
    ]);
    assert_command_success("spawn literal --wait", &literal);
    let literal_brief =
        fs::read_to_string(state_dir(&env.root).join("worker/literal/brief.md")).unwrap();
    assert!(literal_brief.contains("\n--wait\n"), "{literal_brief}");
    assert!(stdout_of(&literal).contains("wait: niles wait"));
}
