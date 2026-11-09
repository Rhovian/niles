#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use std::fs;

#[test]
fn entry_delivers_entry_agent_and_session_header_to_builtin_cli() {
    for (entry, model) in [
        ("reviewer", "sonnet"),
        ("lead", "haiku"),
        ("solo", "sonnet"),
    ] {
        assert_entry(entry, model);
    }
}

#[test]
fn entry_agent_is_chosen_apart_from_standard_primary_and_overridable_within_tiers() {
    let env = start_fixture("niles-foreground-entry-agent", |body| {
        body.replace(
            "entry_agent: claude:haiku",
            "entry_agent: claude:sonnet:high",
        )
    });
    let args_log = env.root.join("args");
    let start = |extra: &[&str]| {
        env.niles(&env.root, &[&["start", "--keep-skills"], extra].concat())
            .env("NILES_TEST_ARGS", &args_log)
            .env_remove("TMUX_PANE")
            .output()
            .unwrap()
    };
    for (extra, model) in [
        (&[][..], "sonnet"),
        (&["--agent", "claude:haiku"][..], "haiku"),
    ] {
        assert_command_success(model, &start(extra));
        let args = fs::read_to_string(&args_log).unwrap();
        assert!(
            args.contains(&format!("--model\0{model}\0")),
            "{model}: {args}"
        );
    }
    assert_failure_contains(
        "unlisted",
        &start(&["--agent", "claude:opus:high"]),
        "agent claude:opus:high is not listed in the manifest tiers",
    );
}

fn assert_entry(entry: &str, model: &str) {
    let env = start_fixture("niles-foreground-header", |body| {
        let mut body = body.replace("entry: lead", &format!("entry: {entry}"));
        let agent = format!(
            "claude:{model}{}",
            if model == "haiku" { "" } else { ":high" }
        );
        for key in ["entry_agent", "primary"] {
            body = body.replace(&format!("{key}: claude:haiku"), &format!("{key}: {agent}"));
        }
        if entry == "solo" {
            body.push_str("\nsolo: {}\n");
        }
        body
    });
    let manifest = state_dir(&env.root).join("manifest.yaml");
    let skill = if entry == "reviewer" {
        env.root.join(format!(".niles/skills/{entry}/SKILL.md"))
    } else {
        env.home.join(format!(".niles/skills/{entry}/SKILL.md"))
    };
    write_skill_document(
        &skill,
        entry,
        "Use when testing entry delivery.",
        "{workspace}\n{manifest}\n{dir}\n{agent}\n{startup_context}\n{unknown}",
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
    let meta = read_json(dir.join("session.json"));
    assert_eq!(meta["entry_skill"], entry, "{entry}");
    assert!(meta.get("setup").is_none(), "{entry}");
    let primary = fs::read_to_string(dir.join("lead.md")).expect(entry);
    assert_eq!(
        primary,
        format!(
            "{}\n{}\n{}\nclaude:{model}{}\nworker: none\n{{unknown}}",
            env.root.display(),
            manifest.display(),
            dir.display(),
            if model == "haiku" { "" } else { ":high" }
        ),
        "{entry}"
    );
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

fn start_fixture(prefix: &str, edit: impl FnOnce(String) -> String) -> TestEnv {
    let env = TestEnv::new(prefix);
    write_workspace_manifest(&env.root, "claude:haiku", "codex", "claude", "codex");
    edit_manifest(&env.root, edit);
    write_executable(
        &env.bin.join("claude"),
        "#!/bin/sh\nprintf '%s\\0' \"$@\" > \"$NILES_TEST_ARGS\"\nprintf '%s' \"${NILES_SETUP-unset}\" > \"$NILES_TEST_ARGS.setup\"\n",
    );
    env
}
