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
