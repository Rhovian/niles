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
    skill(&env.root.join(".niles"), "unreferenced", "", "");
    let manifest = state_dir(&env.root).join("manifest.yaml");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(&manifest, text.replace("worker:", "triage:")).unwrap();
    let output = env.run(&["skills"]);
    assert_command_success("skills", &output);
    let text = stdout_of(&output);
    assert!(text.starts_with("skill: lead (built-in)"));
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
    let preface = primary
        .strip_suffix(body)
        .expect("primary body bytes changed");
    assert!(preface.starts_with("Work from this brief and the code. Do not read other agents' briefs, reports or notes in Niles state unless this brief or a message from the lead names one."));
    assert!(
        preface
            .contains("Deliverables go in the report file; the lead reads it, not your terminal.")
    );
    assert!(preface.ends_with("Report uncertainty as uncertainty.\n\n"));
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
    assert_failure_contains(
        "bounded listing read",
        &env.run(&["skills"]),
        "read 131073 bytes (file size 8589934592 bytes)",
    );
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
            env.run(&["lead", "--keep-skills"])
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
