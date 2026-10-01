use super::support::*;
use std::{io::Write, process::Stdio};

/// Wiring only: `--role` reaches brief composition and each role gets its own fragment on
/// top of the shared contract. What each fragment *says* is asserted in `worker::role`.
#[test]
fn role_selects_which_fragment_the_brief_carries() {
    let env = TestEnv::new("niles-worker-roles");

    let mut briefs = Vec::new();
    for role in ["worker", "reviewer", "security"] {
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

        let brief =
            fs::read_to_string(env.root.join(".niles/worker").join(role).join("brief.md")).unwrap();
        assert!(brief.contains(&format!("You are the {role}")), "{brief}");
        assert!(brief.contains("## Reporting"), "{brief}");
        assert!(brief.contains("done: <short result>; report:"), "{brief}");
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
fn stdin_task_is_composed_with_the_role_and_reporting_contract() {
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

    let brief = fs::read_to_string(env.root.join(".niles/worker/review-file/brief.md")).unwrap();
    assert!(brief.contains("## Reporting"), "{brief}");
    assert!(brief.contains("You are the reviewer"), "{brief}");
    let (_, task_and_reporting) = brief.split_once("## Task\n\n").unwrap();
    let (task, _) = task_and_reporting.split_once("\n\n## Reporting").unwrap();
    assert_eq!(
        task,
        "  Inspect the parser edge cases.\nKeep this line verbatim.\n"
    );
}

#[test]
fn spawn_accepts_literal_flag_text() {
    let env = TestEnv::new("niles-worker-spawn-message-options");

    let literal = env.run(&["spawn", "literal", "--agent", "claude", "--", "--wait"]);
    assert_command_success("spawn literal --wait", &literal);
    let literal_brief =
        fs::read_to_string(env.root.join(".niles/worker/literal/brief.md")).unwrap();
    assert!(literal_brief.contains("\n--wait\n"), "{literal_brief}");
    assert!(stdout_of(&literal).contains("wait: niles wait literal"));
}
