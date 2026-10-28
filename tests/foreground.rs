#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use std::fs;

#[test]
fn entry_delivers_its_binding_primary_and_session_header_to_builtin_cli() {
    for (entry, model, agent_line) in [
        ("reviewer", "sonnet", ""),
        ("lead", "haiku", "lead_agent: claude:haiku"),
        ("solo", "sonnet", "agent: claude:sonnet:high"),
    ] {
        assert_entry(entry, model, agent_line);
    }
}

fn assert_entry(entry: &str, model: &str, agent_line: &str) {
    let env = TestEnv::new("niles-foreground-header");
    write_workspace_manifest(&env.root, "claude:haiku", "codex", "claude", "codex");
    let manifest = state_dir(&env.root).join("manifest.yaml");
    let mut body = fs::read_to_string(&manifest)
        .expect(entry)
        .replace("entry: lead", &format!("entry: {entry}"));
    if entry == "solo" {
        body.push_str("\nsolo: {standard: {primary: claude:sonnet:high, secondaries: []}}\n");
    }
    fs::write(&manifest, body).expect(entry);
    let skill = env.root.join(format!(".niles/skills/{entry}/SKILL.md"));
    if entry == "reviewer" {
        fs::create_dir_all(skill.parent().expect(entry)).expect(entry);
        fs::write(&skill, format!("---\nname: {entry}\ndescription: Use when reviewing.\n---\nentry {{workspace}} {{unknown}}"))
            .expect(entry);
    }
    write_executable(
        &env.bin.join("claude"),
        "#!/bin/sh\nprintf '%s\\0' \"$@\" > \"$NILES_TEST_ARGS\"\nprintf '%s' \"${NILES_SETUP-unset}\" > \"$NILES_TEST_ARGS.setup\"\n",
    );
    let args_log = env.root.join("args");
    let output = env
        .niles(&env.root, &["start", "--keep-skills"])
        .env("NILES_TEST_ARGS", &args_log)
        .env_remove("TMUX_PANE")
        .output()
        .expect(entry);
    assert_command_success(entry, &output);
    let source = if entry == "reviewer" {
        format!("workspace {}", skill.display())
    } else {
        format!(
            "global {}",
            env.home
                .join(format!(".niles/skills/{entry}/SKILL.md"))
                .display()
        )
    };
    assert!(
        stderr_of(&output).contains(&format!("skill: {entry} ({source})")),
        "{entry}"
    );
    assert_eq!(
        fs::read_to_string(env.root.join("args.setup")).expect(entry),
        "unset"
    );
    let args = fs::read_to_string(args_log).expect(entry);
    let args = args
        .strip_suffix('\0')
        .expect(entry)
        .split('\0')
        .collect::<Vec<_>>();
    let dir = args
        .last()
        .expect(entry)
        .strip_prefix("# Niles start session\nsession_dir: ")
        .expect(entry)
        .strip_suffix("\n\nStart the Niles manager session.")
        .expect(entry);
    let dir = std::path::Path::new(dir);
    let meta: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("session.json")).expect(entry)).expect(entry);
    assert_eq!(meta["entry_skill"], entry, "{entry}");
    assert!(meta.get("setup").is_none(), "{entry}");
    let primary = fs::read_to_string(dir.join("lead.md")).expect(entry);
    if entry == "reviewer" {
        assert_eq!(
            primary,
            format!("entry {} {{unknown}}", env.root.display()),
            "{entry}"
        );
    } else {
        for rendered in [
            format!("workspace: {}", env.root.display()),
            format!("manifest: {}", manifest.display()),
            format!("session_dir: {}", dir.display()),
            agent_line.to_owned(),
            "worker: none".to_owned(),
        ] {
            assert!(
                primary.lines().any(|line| line == rendered),
                "{entry}: missing {rendered:?} in {primary}"
            );
        }
    }
    assert!(
        args.windows(2)
            .any(|pair| pair == ["--append-system-prompt", primary.as_str()]),
        "{entry}"
    );
    assert!(
        args.windows(2).any(|pair| pair == ["--model", model]),
        "{entry}"
    );
    assert_failure_contains(
        entry,
        &env.spawn(entry, &["job", "task"]),
        &format!("entry skill {entry} cannot be spawned"),
    );
    assert!(!state_dir(&env.root).join("worker/job").exists(), "{entry}");
}
