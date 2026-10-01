use super::support::*;

/// Wiring only: `--role` reaches brief composition and each role gets its own fragment on
/// top of the shared contract. What each fragment *says* is asserted in `worker::role`.
#[test]
fn role_selects_which_fragment_the_brief_carries() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-roles");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    let mut briefs = Vec::new();
    for role in ["worker", "reviewer", "security"] {
        let spawn = Command::new(niles)
            .args([
                "spawn", role, "--role", role, "--agent", "claude", "Do", "it",
            ])
            .current_dir(&workspace)
            .env("PATH", &path)
            .env("NILES_HOME", &home)
            .env("TMUX_LOG", &tmux_log)
            .env("TMUX", "/tmp/niles-test-tmux,0,0")
            .output()
            .unwrap();
        assert_command_success(&format!("spawn --role {role}"), &spawn);

        let brief = fs::read_to_string(workspace.join(".niles/worker").join(role).join("brief.md"))
            .unwrap();
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
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-task-file");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);
    let task_file = workspace.join("review-task.txt");
    fs::write(&task_file, "Inspect the parser edge cases.").unwrap();

    let spawn = Command::new(niles)
        .args([
            "spawn",
            "review-file",
            "--role",
            "reviewer",
            "--agent",
            "claude",
            "--task-file",
        ])
        .arg(&task_file)
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();
    assert_command_success("spawn --task-file", &spawn);

    let brief = fs::read_to_string(workspace.join(".niles/worker/review-file/brief.md")).unwrap();
    assert!(brief.contains("## Reporting"), "{brief}");
    assert!(brief.contains("You are the reviewer"), "{brief}");
    assert!(brief.contains("Inspect the parser edge cases."), "{brief}");
}
