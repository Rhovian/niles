#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
use common::*;
use std::fs;
use std::os::unix::fs::symlink;
use std::process::{Command, Output};

mod common;

#[test]
fn a_fresh_home_has_no_skill_fallback_or_materialization() {
    let env = TestEnv::with_manifest("skills-fresh-home", "claude");
    fs::remove_dir_all(env.home.join(".niles/skills")).unwrap();
    let output = env.run(&["skills"]);
    assert_command_success("missing disk skill warns", &output);
    assert!(stdout_of(&output).is_empty());
    assert!(stderr_of(&output).contains("warning: skill `lead` has no SKILL.md"));
    assert!(stderr_of(&output).contains("niles skills import"));
    assert!(!env.home.join(".niles/skills").exists());
    assert!(!env.root.join(".niles/skills").exists());
}

#[test]
fn listing_and_launch_show_the_winner_and_deliver_exact_primary_bytes() {
    let env = TestEnv::with_manifest("skills-delivery", "claude");
    let body = "literal {dir} 'quote' \"\r\nbody\n";
    let path = write_skill(
        &env.root.join(".niles"),
        "triage",
        &format!(
            "Use\u{1b}\u{202e}\u{2066}\u{200b}\u{200d}\u{feff}\nwhen {}",
            "é".repeat(250)
        ),
        body,
    );
    write_skill(
        &env.root.join(".niles"),
        "unreferenced",
        "Use when testing discovery",
        "body",
    );
    edit_manifest(&env.root, |text| text.replace("worker:", "triage:"));
    let output = env.run(&["skills"]);
    assert_command_success("skills", &output);
    let text = stdout_of(&output);
    assert!(text.starts_with("skill: lead (global "));
    let row = text
        .lines()
        .find(|line| line.starts_with("skill: triage "))
        .unwrap();
    assert!(row.contains(&format!("workspace {}", path.display())));
    let description = row.split('\t').nth(1).unwrap();
    assert_eq!(description.chars().count(), 200);
    assert_eq!(description, format!("Use when {}", "é".repeat(191)));
    let output = env.spawn("triage", &["job", "task"]);
    assert_command_success("spawn", &output);
    assert!(stdout_of(&output).contains(&format!("skill: triage (workspace {})", path.display())));
    write_executable(
        &env.bin.join("claude"),
        "#!/bin/sh\nprintf '%s\\0' \"$@\" > \"$NILES_TEST_ARGS\"\n",
    );
    let args = env.root.join("args");
    let output = Command::new("sh")
        .arg(state_dir(&env.root).join("worker/job/launch.sh"))
        .env("PATH", path_with_bin(&env.bin))
        .env("NILES_TEST_ARGS", &args)
        .output()
        .unwrap();
    assert_command_success("launch script", &output);
    let args = fs::read_to_string(args).unwrap();
    let args = args.split('\0').collect::<Vec<_>>();
    let primary = args
        .windows(2)
        .find(|pair| matches!(pair, [flag, _] if *flag == "--append-system-prompt"))
        .and_then(|pair| match pair {
            [_, value] => Some(*value),
            _ => None,
        })
        .unwrap();
    assert!(primary.starts_with(&format!("## Skill: triage\n{body}\n")));
}

#[test]
fn discovery_lists_installed_skills_without_bindings() {
    let env = TestEnv::with_manifest("skills-discovery", "claude");
    let manifest = state_dir(&env.root).join("manifest.yaml");
    let tiers = "entry: lead\nentry_agent: claude:sonnet:high\ntiers: {standard: {primary: claude:sonnet:high, secondaries: []}}\n";
    atomic_write(&manifest, tiers).unwrap();
    let local = env.root.join(".niles");
    let global = env.home.join(".niles");
    write_skill(&global, "extra", "Use when global", "global");
    let winner = write_skill(&local, "extra", "Use when local", "local");
    let external = env.root.join("external");
    let target = write_skill(&external, "linked", "Use when linked", "body");
    let link = env.home.join(".niles/skills/linked");
    symlink(target.parent().unwrap(), &link).unwrap();
    let listing = env.run(&["skills"]);
    assert_command_success("discovery without bindings", &listing);
    let text = stdout_of(&listing);
    assert!(text.starts_with("skill: lead (global "));
    assert!(text.contains(&format!("skill: extra (workspace {})", winner.display())));
    assert!(text.contains("skill: linked (global "));
    assert!(text.contains("skill: worker (global "));
    symlink(target.parent().unwrap(), local.join("skills/linked")).unwrap();
    let listing = env.run(&["skills"]);
    assert_command_success("discovered workspace symlink warns", &listing);
    assert!(stderr_of(&listing).contains("warning:"));
    assert!(stderr_of(&listing).contains("without symlinks"));
    assert!(stdout_of(&listing).contains("skill: worker "));
    assert!(!stdout_of(&listing).contains("skill: linked "));
}

#[test]
fn bad_directories_warn_without_hiding_valid_skills_or_allowing_launch() {
    let env = TestEnv::with_manifest("skills-warnings", "claude");
    let local = env.root.join(".niles");
    write_skill(&local, "foo.bak", "Use when invalid", "body");
    let pointer = local.join("skills/broken/SKILL.ref");
    fs::create_dir_all(pointer.parent().unwrap()).unwrap();
    atomic_write(&pointer, "broken ref").unwrap();
    edit_manifest(&env.root, |body| {
        format!("{body}broken: {{}}\nghost: {{}}\n")
    });
    let listing = env.run(&["skills"]);
    assert_command_success("invalid discovered skills warn", &listing);
    let warnings = stderr_of(&listing);
    for cause in ["foo.bak", "SKILL.ref", "skill `ghost` has no SKILL.md"] {
        assert!(warnings.contains(cause), "{warnings}");
    }
    assert!(stdout_of(&listing).contains("skill: worker "));
    assert!(!stdout_of(&listing).contains("skill: broken "));
    assert!(!stdout_of(&listing).contains("skill: ghost "));
    let launch = env.spawn("broken", &["bad", "Task"]);
    assert_failure_contains("invalid explicit launch", &launch, "invalid");
    assert!(!state_dir(&env.root).join("worker/bad").exists());
}

#[test]
fn invalid_primaries_fail_before_worker_or_session_creation() {
    let env = TestEnv::with_manifest("skills-bounds", "codex");
    let path = write_skill(&env.root.join(".niles"), "worker", "valid", "body");
    atomic_write(&path, "malformed frontmatter").unwrap();
    assert_command_success("load checks existence only", &env.run(&["models"]));
    assert_failure_contains(
        "malformed skill",
        &env.spawn("worker", &["job", "task"]),
        path.to_str().unwrap(),
    );
    assert!(!state_dir(&env.root).join("worker/job").exists());
    for (name, body, error) in [
        ("worker", " \n", "empty body"),
        ("lead", " \n", "empty body"),
        ("worker", "bad\0text", "NUL"),
        ("lead", "bad\0text", "NUL"),
    ] {
        let path = write_skill(&env.root.join(".niles"), name, "valid", body);
        let output = if name == "worker" {
            env.spawn(name, &["job", "--worktree", "untouched", "task"])
        } else {
            env.run(&["start", "--keep-skills"])
        };
        assert_failure_contains("invalid primary", &output, error);
        assert!(!state_dir(&env.root).join("worker/job").exists());
        assert!(!state_dir(&env.root).join("sessions").exists());
        fs::remove_file(path).unwrap();
    }
    edit_manifest(&env.root, |text| text.replace("worker:", "reveiwer:"));
    assert_failure_contains(
        "unresolved binding",
        &env.spawn("worker", &["job", "task"]),
        "skill `reveiwer` has no SKILL.md",
    );
    assert!(!state_dir(&env.root).join("worker/job").exists());
}

fn bind_worker(env: &TestEnv, extra: &str) {
    edit_manifest(&env.root, |body| {
        let mut manifest: serde_json::Value = serde_saphyr::from_str(&body).unwrap();
        manifest.as_object_mut().unwrap().insert(
            "worker".to_owned(),
            serde_saphyr::from_str::<serde_json::Value>(extra).unwrap(),
        );
        serde_saphyr::to_string(&manifest).unwrap()
    });
}

fn discovery(env: &TestEnv, args: &[&str]) -> Output {
    env.niles(&env.home, &[])
        .env("NILES_AGENT_DIR", state_dir(&env.root).join("worker/job"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn explicit_injection_and_named_loading_in_any_session() {
    let env = TestEnv::with_manifest("skill-loading", "claude");
    let root = &env.root.join(".niles");
    write_skill(root, "forced", "Use when forced", "forced body");
    bind_worker(
        &env,
        "  always: [worker, solo, solo]\n  on_stage: [reviewer]\n",
    );
    let output = env.spawn(
        "worker",
        &[
            "job", "--with", "solo", "--with", "forced", "--with", "forced", "--with", "worker",
            "task",
        ],
    );
    assert_command_success("spawn", &output);
    assert!(!stderr_of(&output).contains("warning:"));
    for prefix in [
        "always_skill: solo",
        "on_stage_skill: reviewer",
        "with_skill: forced",
    ] {
        assert!(stdout_of(&output).contains(prefix));
    }
    let dir = state_dir(&env.root).join("worker/job");
    let meta = read_json(dir.join("meta.json"));
    assert_eq!(meta["on_stage"], serde_json::json!(["reviewer"]));
    let launch = fs::read_to_string(dir.join("launch.sh")).unwrap();
    assert!(launch.contains("## Skill: solo"));
    assert!(launch.contains("## Skill: forced\nforced body"));
    let positions = ["worker", "solo", "forced"]
        .map(|name| launch.find(&format!("## Skill: {name}\n")).unwrap());
    assert!(
        positions
            .windows(2)
            .all(|pair| matches!(pair, [a, b] if a < b))
    );
    for name in ["worker", "solo", "forced"] {
        assert_eq!(
            launch.matches(&format!("## Skill: {name}\n")).count(),
            1,
            "{name}"
        );
    }
    write_skill(
        root,
        "codex-trim",
        "Use when loading workspace skills",
        "workspace body\n",
    );
    write_skill(
        &env.home.join(".niles"),
        "niles",
        "Use when loading global skills",
        "global body\n",
    );
    let listing = discovery(&env, &["skills"]);
    assert_command_success(
        "worker installed skill listing outside project cwd",
        &listing,
    );
    assert!(stdout_of(&listing).contains("skill: codex-trim (workspace "));
    assert!(stdout_of(&listing).contains("skill: niles (global "));
    for worker in [true, false] {
        for (name, body) in [
            ("codex-trim", "workspace body\n"),
            ("niles", "global body\n"),
        ] {
            let output = if worker {
                discovery(&env, &["agent", "skill", name])
            } else {
                env.niles(&env.root, &["agent", "skill", name])
                    .output()
                    .unwrap()
            };
            assert_command_success("named skill load", &output);
            assert!(stdout_of(&output).starts_with(&format!("skill: {name} (")));
            assert!(stdout_of(&output).ends_with(body));
            assert!(!stdout_of(&output).contains("---\nname:"));
        }
    }
}
