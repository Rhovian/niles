use super::support::*;
use std::{io::Write, process::Stdio};

#[test]
fn spawn_any_skill_records_mechanical_decisions_and_from_headers() {
    let env = TestEnv::with_manifest("niles-worker-design-record", "claude");
    let session = state_dir(&env.root).join("sessions/lead");
    fs::create_dir_all(&session).unwrap();
    let meta = serde_json::json!({
        "id": "lead", "entry_skill": "lead", "agent": "claude", "created_at": "2026-01-02T03:04:05Z",
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
        "--skill",
        "reviewer",
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
    assert!(!header.contains("from_report:"));

    let designer = env.spawn("research", &["designer", "design"]);
    assert_command_success("designer", &designer);
    fs::write(workers.join("designer/status.log"), "done: agreed record\n").unwrap();
    let spawned = env.spawn("design", &["implementation", "--from", "designer", "task"]);
    assert_command_success("worker with design", &spawned);
    let body = fs::read_to_string(workers.join("implementation/brief.md")).unwrap();
    assert!(body.contains(&format!(
        "report_file: {}\nfrom_report: {}\n",
        workers.join("implementation/report.md").display(),
        workers.join("designer/report.md").display()
    )));
}

#[test]
fn skill_selects_primary_instructions_outside_the_opening_turn() {
    let env = TestEnv::with_manifest("niles-worker-skills", "claude");

    for skill in ["design", "reviewer", "security", "research", "worker"] {
        let spawn = env.spawn(skill, &[skill, "Do", "it"]);
        assert_command_success(&format!("spawn --skill {skill}"), &spawn);

        let dir = state_dir(&env.root).join(format!("worker/{skill}"));
        let brief = fs::read_to_string(dir.join("brief.md")).unwrap();
        assert!(
            brief.starts_with(&format!("# Niles {skill} brief\n")),
            "{brief}"
        );
        let script = fs::read_to_string(dir.join("launch.sh")).unwrap();
        assert!(script.contains("'--append-system-prompt'"), "{script}");
    }
}

#[test]
fn stdin_task_preserves_multiline_text() {
    let env = TestEnv::with_manifest("niles-worker-stdin-task", "claude");
    let mut child = env
        .spawn_command("reviewer", &["review-file", "-"])
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
    let env = TestEnv::with_manifest("niles-worker-spawn-message-options", "claude");

    let literal = env.spawn("research", &["literal", "--", "--wait"]);
    assert_command_success("spawn literal --wait", &literal);
    let literal_brief =
        fs::read_to_string(state_dir(&env.root).join("worker/literal/brief.md")).unwrap();
    assert!(literal_brief.contains("\n--wait\n"), "{literal_brief}");
    assert!(stdout_of(&literal).contains("wait: niles wait"));
}
