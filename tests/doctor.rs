#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use std::{fs, path::Path, process::Command};

#[test]
fn doctor_reports_binary_identity() {
    let workspace = temp_workspace("niles-doctor-test");
    let home = niles_home(&workspace);
    write_workspace_manifest(&workspace, "claude", "codex", "claude", "claude");

    let output = niles_bare(&workspace, &home)
        .arg("doctor")
        .output()
        .unwrap();

    assert_command_success("doctor", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(concat!("binary: niles ", env!("CARGO_PKG_VERSION"), " (")));
    assert!(stdout.contains("git_hash: "));
    assert!(stdout.contains("built_at: "));
    assert!(stdout.contains("dev_mode: no"));
}

#[test]
fn doctor_reports_manifest_binaries_once_with_versions() {
    let env = TestEnv::with_tmux(
        "niles-doctor-agents",
        "#!/bin/sh\nprintf 'tmux 3.5\\nsecond line\\n'\n",
    );
    write_workspace_manifest(&env.root, "claude", "codex", "claude", "claude");
    write_executable(
        &env.bin.join("claude"),
        "#!/bin/sh\nprintf 'Claude CLI 2.1.288\\nsecond line\\n'\n",
    );
    write_executable(
        &env.bin.join("codex"),
        "#!/bin/sh\nprintf 'Codex CLI 0.160.0\\n'\n",
    );

    let output = env.run(&["doctor"]);

    assert_command_success("doctor agents", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains("tmux: tmux 3.5\n"), "{stdout}");
    assert!(stdout.contains(&format!(
        "agent claude: {} — Claude CLI 2.1.288; tested 2.1.288",
        env.bin.join("claude").display()
    )));
    assert!(stdout.contains(&format!(
        "agent codex: {} — Codex CLI 0.160.0; tested 0.160.0",
        env.bin.join("codex").display()
    )));
    assert_eq!(stdout.matches("agent claude:").count(), 1);
    assert!(!stdout.contains("second line"));
}

#[test]
fn doctor_reports_custom_missing_and_unavailable_versions() {
    let env = TestEnv::new("niles-doctor-custom");
    write_workspace_manifest(&env.root, "custom", "codex", "lead", "claude");
    fs::write(
        env.root.join("niles.yaml"),
        "agents:\n  custom:\n    binary: custom-cli\n  codex:\n    binary: missing-cli\n",
    )
    .unwrap();
    write_executable(
        &env.bin.join("custom-cli"),
        "#!/bin/sh\nprintf 'version failed\\n' >&2\nexit 1\n",
    );

    let output = env.run(&["doctor"]);

    assert_command_success("doctor custom", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains(&format!(
        "agent custom-cli: {} — version unavailable: version failed",
        env.bin.join("custom-cli").display()
    )));
    assert!(stdout.contains("agent missing-cli: not found on PATH"));
    assert!(!stdout.contains("agent lead:"));
    assert!(!stdout.contains("version failed; tested"));
}

#[test]
fn doctor_without_manifest_reports_absence() {
    let env = TestEnv::new("niles-doctor-no-manifest");
    let output = env.run(&["doctor"]);
    assert_command_success("doctor no manifest", &output);
    assert!(stdout_of(&output).contains("agents: no workspace manifest"));
}

#[test]
fn doctor_dirty_source_tree_never_reports_stale_no() {
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
    git(
        &workspace,
        &[
            "-c",
            "user.name=Niles Test",
            "-c",
            "user.email=niles@example.invalid",
            "commit",
            "-m",
            "initial",
        ],
    );
    fs::write(
        workspace.join("src/main.rs"),
        "fn main() { println!(\"dirty\"); }\n",
    )
    .unwrap();

    let output = niles_bare(&workspace, &niles_home(&workspace))
        .arg("doctor")
        .output()
        .unwrap();

    assert_command_success("doctor dirty", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("dev_mode: yes"));
    assert!(stdout.contains("working_tree: dirty"));
    assert!(stdout.contains("stale: unknown (working tree dirty)"));
    assert!(!stdout.contains("stale: no"));
}

#[test]
fn version_includes_build_identity() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let output = Command::new(niles).arg("--version").output().unwrap();

    assert_command_success("--version", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(concat!("niles ", env!("CARGO_PKG_VERSION"), " (")));
    assert!(stdout.contains("built "));
}

fn git(workspace: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .unwrap();
    assert_command_success(&format!("git {}", args.join(" ")), &output);
}
