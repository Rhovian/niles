use super::support::*;

#[test]
fn skill_selects_primary_instructions_outside_the_opening_turn() {
    let env = TestEnv::with_manifest("niles-worker-skills", "claude");

    for skill in ["niles", "reviewer", "codex-trim", "solo", "worker"] {
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
        assert!(!script.contains("'--mcp-config'"), "{script}");
    }
}

#[test]
fn replacement_skills_discover_the_binary_contract_for_every_family() {
    let env = TestEnv::new("niles-worker-contract");
    let skill_dir = env.root.join(".niles/skills/custom");
    fs::create_dir_all(&skill_dir).unwrap();
    atomic_write(
        skill_dir.join("SKILL.md"),
        "---\nname: custom\ndescription: Use when answering questions.\n---\nAnswer questions.\n",
    )
    .unwrap();
    for (family, agent) in [
        ("claude", "claude"),
        ("codex", "codex"),
        ("pi", "pi:deepseek/deepseek-v4.1-flash:high"),
    ] {
        write_executable(&env.bin.join(family), "#!/bin/sh\nexit 0\n");
        write_manifest(&env.root, [("lead", "claude"), ("custom", agent)]);
        let output = env.spawn("custom", &[family, "Answer this"]);
        assert_command_success(family, &output);
        let dir = state_dir(&env.root).join(format!("worker/{family}"));
        let script = fs::read_to_string(dir.join("launch.sh")).unwrap();
        assert!(script.contains("Answer questions."));
        assert!(!script.contains("Niles orchestration contract"));
        let brief = fs::read_to_string(dir.join("brief.md")).unwrap();
        assert!(brief.contains("Read the reporting contract in `niles agent status --help`."));
        assert!(
            brief.contains("`niles skills` lists skills; `niles agent skill <name>` loads one.")
        );
    }
}

#[test]
fn stdin_task_preserves_multiline_text() {
    let env = TestEnv::with_manifest("niles-worker-stdin-task", "claude");
    let spawn = output_with_stdin(
        env.spawn_command("reviewer", &["review-file", "-"]),
        b"  Inspect the parser edge cases.\nKeep this line verbatim.\n\n",
    );
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
