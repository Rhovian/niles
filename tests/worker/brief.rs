use super::support::*;

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
fn task_file_is_composed_with_the_role_and_reporting_contract() {
    let env = TestEnv::new("niles-worker-task-file");
    let task_file = env.root.join("review-task.txt");
    fs::write(&task_file, "Inspect the parser edge cases.").unwrap();

    let spawn = env
        .niles(
            &env.root,
            &[
                "spawn",
                "review-file",
                "--role",
                "reviewer",
                "--agent",
                "claude",
                "--task-file",
            ],
        )
        .arg(&task_file)
        .output()
        .unwrap();
    assert_command_success("spawn --task-file", &spawn);

    let brief = fs::read_to_string(env.root.join(".niles/worker/review-file/brief.md")).unwrap();
    assert!(brief.contains("## Reporting"), "{brief}");
    assert!(brief.contains("You are the reviewer"), "{brief}");
    assert!(brief.contains("Inspect the parser edge cases."), "{brief}");
}
