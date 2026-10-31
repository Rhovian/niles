#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;
use common::*;
use std::{fs, path::Path, process::Command};

fn skill(root: &Path, name: &str, description: &str, body: &str) -> std::path::PathBuf {
    let path = root.join("skills").join(name).join("SKILL.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!(
            "---\nname: {name}\ndescription: {}\n---\n{body}",
            serde_json::to_string(description).unwrap()
        ),
    )
    .unwrap();
    path
}

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
    let path = skill(
        &env.root.join(".niles"),
        "triage",
        &format!(
            "Use\u{1b}\u{202e}\u{2066}\u{200b}\u{200d}\u{feff}\nwhen {}",
            "é".repeat(250)
        ),
        body,
    );
    skill(
        &env.root.join(".niles"),
        "unreferenced",
        "Use when testing discovery",
        "body",
    );
    let manifest = state_dir(&env.root).join("manifest.yaml");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(&manifest, text.replace("worker:", "triage:")).unwrap();
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
        .find(|pair| pair[0] == "--append-system-prompt")
        .unwrap()[1];
    assert!(primary.starts_with(&format!("## Skill: triage\n{body}\n")));
}

#[test]
fn discovery_lists_installed_skills_without_bindings() {
    let env = TestEnv::with_manifest("skills-discovery", "claude");
    let manifest = state_dir(&env.root).join("manifest.yaml");
    let tiers = "entry: lead\nentry_agent: claude:sonnet:high\ntiers: {standard: {primary: claude:sonnet:high, secondaries: []}}\n";
    fs::write(&manifest, tiers).unwrap();
    let local = env.root.join(".niles");
    let global = env.home.join(".niles");
    skill(&global, "extra", "Use when global", "global");
    let winner = skill(&local, "extra", "Use when local", "local");
    let external = env.root.join("external");
    let target = skill(&external, "linked", "Use when linked", "body");
    let link = env.home.join(".niles/skills/linked");
    std::os::unix::fs::symlink(target.parent().unwrap(), &link).unwrap();
    let listing = env.run(&["skills"]);
    assert_command_success("discovery without bindings", &listing);
    let text = stdout_of(&listing);
    assert!(text.starts_with("skill: lead (global "));
    assert!(text.contains(&format!("skill: extra (workspace {})", winner.display())));
    assert!(text.contains("skill: linked (global "));
    assert!(text.contains("skill: worker (global "));
    std::os::unix::fs::symlink(target.parent().unwrap(), local.join("skills/linked")).unwrap();
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
    skill(&local, "foo.bak", "Use when invalid", "body");
    let pointer = local.join("skills/broken/SKILL.ref");
    fs::create_dir_all(pointer.parent().unwrap()).unwrap();
    fs::write(&pointer, "broken ref").unwrap();
    let manifest = state_dir(&env.root).join("manifest.yaml");
    let body = fs::read_to_string(&manifest).unwrap();
    fs::write(manifest, format!("{body}broken: {{}}\nghost: {{}}\n")).unwrap();
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
    let path = skill(&env.root.join(".niles"), "worker", "valid", "body");
    fs::write(&path, "malformed frontmatter").unwrap();
    assert_command_success("load checks existence only", &env.run(&["models"]));
    assert_failure_contains(
        "malformed skill",
        &env.spawn("worker", &["job", "task"]),
        path.to_str().unwrap(),
    );
    assert!(!state_dir(&env.root).join("worker/job").exists());
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(8 * 1024 * 1024 * 1024)
        .unwrap();
    let listing = env.run(&["skills"]);
    assert_command_success("oversized bound skill warns", &listing);
    assert!(stderr_of(&listing).contains("warning:"));
    assert!(stderr_of(&listing).contains("read 131073 bytes (file size 8589934592 bytes)"));
    assert!(!stdout_of(&listing).contains("skill: worker "));
    assert!(stdout_of(&listing).contains("skill: reviewer "));
    for (name, body, error) in [
        ("worker", " \n".into(), "empty body"),
        ("lead", " \n".into(), "empty body"),
        ("worker", "bad\0text".into(), "NUL"),
        ("lead", "bad\0text".into(), "NUL"),
        ("worker", "x".repeat(65536), "maximum is 65536"),
        ("lead", "x".repeat(65537), "maximum is 65536"),
        ("worker", "\u{1b}".repeat(24000), "encoded primary"),
        ("lead", "\u{1b}".repeat(24000), "encoded primary"),
    ] {
        let path = skill(&env.root.join(".niles"), name, "valid", &body);
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
    let manifest = state_dir(&env.root).join("manifest.yaml");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(&manifest, text.replace("worker:", "reveiwer:")).unwrap();
    assert_failure_contains(
        "unresolved binding",
        &env.spawn("worker", &["job", "task"]),
        "skill `reveiwer` has no SKILL.md",
    );
    assert!(!state_dir(&env.root).join("worker/job").exists());
}

fn bind_extras(env: &TestEnv, extra: &str) {
    let path = state_dir(&env.root).join("manifest.yaml");
    let body = fs::read_to_string(&path).unwrap();
    let body = body
        .replace("worker: {}\n", "worker:\n")
        .replace("worker:\n", &format!("worker:\n{extra}"));
    fs::write(path, body).unwrap();
}

fn discovery(env: &TestEnv, args: &[&str]) -> std::process::Output {
    env.niles(&env.home, &[])
        .env("NILES_AGENT_DIR", state_dir(&env.root).join("worker/job"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn menu_union_snapshot_discovery_and_explicit_injection() {
    let env = TestEnv::with_manifest("menu-discovery", "claude");
    let root = &env.root.join(".niles");
    let description = format!(
        "Use WHEN this\n task applies\u{1b}\u{202e}\u{2066}\u{200b} {}",
        "é".repeat(250)
    );
    for name in ["alpha", "beta", "forced"] {
        skill(root, name, &description, &format!("{name} body"));
    }
    bind_extras(
        &env,
        "  skills: [alpha, alpha]\n  always: [worker, research, research]\n  on_from: [research, design, design]\n  on_stage: [reviewer]\n",
    );
    let output = env.spawn(
        "worker",
        &[
            "job", "--menu", "beta", "--menu", "alpha", "--with", "research", "--with", "forced",
            "--with", "forced", "--with", "worker", "task",
        ],
    );
    assert_command_success("spawn", &output);
    assert!(!stderr_of(&output).contains("warning:"));
    for prefix in [
        "menu_skill: alpha",
        "always_skill: research",
        "on_from_skill: design",
        "on_stage_skill: reviewer",
        "with_skill: forced",
    ] {
        assert!(stdout_of(&output).contains(prefix));
    }
    let dir = state_dir(&env.root).join("worker/job");
    let meta: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("meta.json")).unwrap()).unwrap();
    for (key, names) in [
        ("menu", vec!["alpha", "beta"]),
        ("always", vec!["worker", "research", "research"]),
        ("on_from", vec!["research", "design", "design"]),
        ("on_stage", vec!["reviewer"]),
        ("with", vec!["research", "forced", "forced", "worker"]),
    ] {
        assert_eq!(meta[key], serde_json::json!(names), "{key}");
    }
    let brief = fs::read_to_string(dir.join("brief.md")).unwrap();
    assert_eq!(brief.matches("- alpha:").count(), 1);
    assert!(brief.contains("- beta: Use WHEN this task applies "));
    let launch = fs::read_to_string(dir.join("launch.sh")).unwrap();
    assert!(launch.contains("## Skill: research"));
    assert!(launch.contains("## Skill: forced\nforced body"));
    assert!(!launch.contains("## Skill: design"));
    fs::write(dir.join("status.log"), "done: ready\n").unwrap();
    let args = [
        "followup", "--from", "job", "--with", "research", "--with", "forced", "--with", "forced",
        "--with", "worker", "task",
    ];
    assert_command_success("from spawn", &env.spawn("worker", &args));
    let launch =
        fs::read_to_string(state_dir(&env.root).join("worker/followup/launch.sh")).unwrap();
    let positions = ["worker", "research", "design", "forced"]
        .map(|name| launch.find(&format!("## Skill: {name}\n")).unwrap());
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    for name in ["worker", "research", "design", "forced"] {
        assert_eq!(
            launch.matches(&format!("## Skill: {name}\n")).count(),
            1,
            "{name}"
        );
    }
    // Editing the binding cannot add permission to the running worker's discovery lists.
    let manifest = state_dir(&env.root).join("manifest.yaml");
    let body = fs::read_to_string(&manifest)
        .unwrap()
        .replace("skills: [alpha, alpha]", "skills: [forced]");
    fs::write(manifest, body).unwrap();
    let output = discovery(&env, &["agent", "skills"]);
    assert_command_success("menu discovery", &output);
    assert!(stdout_of(&output).starts_with("alpha:"));
    assert!(!stdout_of(&output).contains("forced:"));
    for text in [&brief, &stdout_of(&output)] {
        assert!(!text.contains(['\u{1b}', '\u{202e}', '\u{2066}', '\u{200b}']));
        let row = text.lines().find(|line| line.contains("alpha: ")).unwrap();
        let description = row
            .split_once(": ")
            .unwrap()
            .1
            .rsplit_once(" (workspace ")
            .unwrap()
            .0;
        assert_eq!(description.chars().count(), 200);
    }
    for name in ["alpha", "research", "design", "reviewer", "forced"] {
        let output = discovery(&env, &["agent", "skill", name]);
        assert_command_success("skill discovery", &output);
        assert!(stdout_of(&output).starts_with(&format!("skill: {name} (")));
        assert!(!stdout_of(&output).contains("---\nname:"));
    }
    assert_failure_contains(
        "unlisted skill",
        &discovery(&env, &["agent", "skill", "security"]),
        "valid skills: alpha, beta, worker, research, research, research, design, design, reviewer, research, forced, forced, worker",
    );
}

#[test]
fn menu_errors_caps_and_warning_precede_state_creation() {
    let env = TestEnv::with_manifest("menu-errors", "claude");
    let root = &env.root.join(".niles");
    let dir = state_dir(&env.root).join("worker/job");
    let reject = |args: &[&str], message| {
        let output = env.spawn("worker", args);
        assert_failure_contains("rejected spawn", &output, message);
        assert!(!stderr_of(&output).contains("warning:"));
        assert!(!dir.exists());
    };
    for (description, warning) in [
        ("Whenever useful", true),
        ("Use when useful", false),
        ("Use WHEN useful", false),
        ("some_when text", true),
        ("somewhen text", true),
    ] {
        skill(root, "guide", description, "body");
        let output = env.spawn("worker", &["job", "--menu", "guide", "task"]);
        assert_command_success("description warning", &output);
        assert_eq!(
            stderr_of(&output).contains("warning: menu skill guide"),
            warning
        );
        fs::remove_dir_all(state_dir(&env.root).join("worker/job")).unwrap();
    }
    for (name, message) in [("absent", "has no SKILL.md"), ("../x", "only github.com")] {
        reject(&["job", "--menu", name, "task"], message);
    }
    for description in ["".to_owned(), "x".repeat(1025)] {
        skill(root, "guide", &description, "body");
        reject(
            &["job", "--menu", "guide", "task"],
            "description must be nonempty",
        );
    }
    let names = (0..17).map(|n| format!("s{n}")).collect::<Vec<_>>();
    for name in &names {
        skill(root, name, "Use when needed", "body");
    }
    let mut args = vec!["job"];
    for name in &names[..16] {
        args.extend(["--menu", name]);
    }
    args.extend(["--menu", &names[0], "task"]);
    assert_command_success(
        "seventeen names, sixteen unique skills",
        &env.spawn("worker", &args),
    );
    fs::remove_dir_all(state_dir(&env.root).join("worker/job")).unwrap();
    args.pop();
    args.extend(["--menu", &names[16], "task"]);
    reject(&args, "maximum is 16");
    skill(root, "guide", "Whenever useful", "body");
    assert_command_success(
        "ordinary research menu at spawn",
        &env.spawn("worker", &["job", "--menu", "research", "task"]),
    );
    fs::remove_dir_all(&dir).unwrap();
    let path = state_dir(&env.root).join("manifest.yaml");
    bind_extras(&env, "  skills: [research]\n");
    assert_command_success("ordinary research menu at load", &env.run(&["models"]));
    for name in ["large0", "large1", "large2", "large3"] {
        skill(root, name, "Use when needed", &"x".repeat(16384));
    }
    assert_command_success(
        "duplicate bodies share one budget slot",
        &env.spawn(
            "worker",
            &[
                "job", "--with", "large0", "--with", "large0", "--with", "large0", "--with",
                "large0", "task",
            ],
        ),
    );
    fs::remove_dir_all(&dir).unwrap();
    bind_extras(&env, "  on_stage: [large0, large1]\n");
    assert_command_success("exact stage cap", &env.spawn("worker", &["job", "task"]));
    fs::remove_dir_all(&dir).unwrap();
    skill(root, "large1", "Use when needed", &"x".repeat(16385));
    reject(
        &["job", "--menu", "guide", "task"],
        "on_stage bodies exceed 32768",
    );
    let body = fs::read_to_string(&path).unwrap().replace(
        "on_stage: [large0, large1]",
        "always: [large0, large1, large2, large3]",
    );
    fs::write(&path, body).unwrap();
    reject(&["job", "--menu", "guide", "task"], "maximum is 65536");
    reject(
        &["job", "--on-send", "large", "task"],
        "unexpected argument",
    );
}
