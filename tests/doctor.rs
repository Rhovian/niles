#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use std::fs;

#[test]
fn doctor_reports_binary_identity() {
    let workspace = temp_workspace("niles-doctor-test");
    let home = niles_home(&workspace);

    let output = niles_bare(&workspace, &home)
        .arg("doctor")
        .output()
        .unwrap();

    assert_command_success("doctor", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains(concat!("binary: niles ", env!("CARGO_PKG_VERSION"), " (")));
    assert!(stdout.contains("git_hash: "));
    assert!(stdout.contains("built_at: "));
    assert!(stdout.contains("dev_mode: no"));
    assert!(stdout.contains("agents: no workspace manifest"));
    let output = run_bare(&workspace, &["--version"]);
    assert_success_contains(
        "--version",
        &output,
        concat!("niles ", env!("CARGO_PKG_VERSION"), " ("),
    );
    assert!(stdout_of(&output).contains("built "));
}

#[test]
fn doctor_reports_manifest_binaries_once_with_versions() {
    let env = TestEnv::with_tmux(
        "niles-doctor-agents",
        "#!/bin/sh\nprintf 'tmux 3.5\\nsecond line\\n'\n",
    );
    write_workspace_manifest(
        &env.root,
        "pi:z-ai/glm-5.3-flash:high",
        "codex",
        "claude:opus:high",
        "claude",
    );
    write_executable(
        &env.bin.join("claude"),
        "#!/bin/sh\nprintf 'Claude CLI 1.0.0\\nsecond line\\n'\n",
    );
    write_executable(
        &env.bin.join("codex"),
        "#!/bin/sh\nprintf 'Codex CLI 1.0.0\\n'\n",
    );
    write_executable(&env.bin.join("pi"), "#!/bin/sh\nprintf 'Pi CLI 1.0.0\\n'\n");

    let output = env.run(&["doctor"]);

    assert_command_success("doctor agents", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains("tmux: tmux 3.5\n"), "{stdout}");
    assert!(stdout.contains(&format!(
        "agent claude: {} — Claude CLI 1.0.0; tested ",
        env.bin.join("claude").display()
    )));
    assert!(stdout.contains(&format!(
        "agent codex: {} — Codex CLI 1.0.0; tested ",
        env.bin.join("codex").display()
    )));
    assert!(stdout.contains("agent pi:"), "{stdout}");
    assert_eq!(stdout.matches("agent claude:").count(), 1);
    assert!(!stdout.contains("second line"));
}

#[test]
fn doctor_reports_missing_and_unavailable_builtin_versions() {
    let env = TestEnv::new("niles-doctor-unavailable");
    write_workspace_manifest(&env.root, "claude", "codex", "claude", "claude");
    fs::remove_file(env.bin.join("codex")).unwrap();
    write_executable(
        &env.bin.join("claude"),
        "#!/bin/sh\nprintf 'version failed\\n' >&2\nexit 1\n",
    );

    let output = env
        .niles(&env.root, &["doctor"])
        .env("PATH", &env.bin)
        .output()
        .unwrap();

    assert_command_success("doctor unavailable", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains(&format!(
        "agent claude: {} — version unavailable: version failed",
        env.bin.join("claude").display()
    )));
    assert!(stdout.contains("agent codex: not found on PATH"));
    assert!(!stdout.contains("agent lead:"));
}

#[test]
fn doctor_dirty_source_tree_ignores_inherited_repository_routing() {
    let workspace = temp_workspace("niles-doctor-dirty-test");
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        r#"[package]
name = "niles"
version = "0.1.0"
edition = "2024"
"#,
    )
    .unwrap();
    fs::write(workspace.join("src/main.rs"), "fn main() {}\n").unwrap();
    git(&workspace, &["init"]);
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "-m", "initial"]);
    let caller = temp_workspace("niles-doctor-caller");
    git(&workspace, &["clone", ".", caller.to_str().unwrap()]);
    fs::write(
        workspace.join("src/main.rs"),
        "fn main() { println!(\"dirty\"); }\n",
    )
    .unwrap();

    let output = niles_bare(&workspace, &niles_home(&workspace))
        .arg("doctor")
        .env("GIT_DIR", caller.join(".git"))
        .env("GIT_WORK_TREE", &caller)
        .output()
        .unwrap();

    assert_command_success("doctor dirty", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains("dev_mode: yes"));
    assert!(stdout.contains("working_tree: dirty"));
    assert!(stdout.contains("stale: unknown (working tree dirty)"));
    assert!(!stdout.contains("stale: no"));
}
