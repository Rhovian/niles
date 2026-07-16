mod common;

use common::*;
use std::{fs, path::Path, process::Command};

#[test]
fn doctor_reports_binary_identity_and_workspace_schema_state() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-doctor-test");
    let home = niles_home(&workspace);
    write_workspace_manifest(&workspace, "claude", "claude", "codex", "claude", "test");
    fs::create_dir_all(home.join("runs")).unwrap();
    fs::write(home.join("runs/index.json"), "{ invalid global index").unwrap();

    let output = Command::new(niles)
        .arg("doctor")
        .current_dir(&workspace)
        .env("NILES_HOME", home)
        .output()
        .unwrap();

    assert_command_success("doctor", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("binary: niles 0.1.0 ("));
    assert!(stdout.contains("git_hash: "));
    assert!(stdout.contains("built_at: "));
    assert!(stdout.contains("schema: 2"));
    assert!(stdout.contains("schemas[1]{kind,path,status}:"));
    assert!(stdout.contains("workspace manifest,.niles/manifest.yaml,current schema 2"));
    assert!(!stdout.contains("global Niles index"));
    assert!(stdout.contains("dev_mode: no"));
}

#[test]
fn doctor_reports_workspace_artifact_classes_nonzero() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-doctor-artifacts-test");
    let home = niles_home(&workspace);

    fs::create_dir_all(workspace.join(".niles/worker/worker-1")).unwrap();
    fs::create_dir_all(workspace.join(".niles/sessions/session-1")).unwrap();
    fs::create_dir_all(workspace.join(".niles/capabilities")).unwrap();
    fs::write(
        workspace.join(".niles/manifest.yaml"),
        "manager: claude\nplanner: claude\nworker: codex\nreviewer: claude\nvalidation_command: test\n",
    )
    .unwrap();
    fs::write(workspace.join(".niles/worker/worker-1/meta.json"), "{}").unwrap();
    fs::write(workspace.join(".niles/worker/worker-1.json"), "{}").unwrap();
    fs::write(
        workspace.join(".niles/sessions/session-1/session.json"),
        "{}",
    )
    .unwrap();
    fs::write(workspace.join(".niles/capabilities/codex.json"), "{}").unwrap();

    let output = Command::new(niles)
        .arg("doctor")
        .current_dir(&workspace)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("workspace manifest,.niles/manifest.yaml,older schema 1"));
    assert!(stdout.contains("worker metadata,.niles/worker/worker-1/meta.json,older schema 1"));
    assert!(!stdout.contains(".niles/worker/worker-1.json"));
    assert!(stdout.contains(
        "manager session metadata,.niles/sessions/session-1/session.json,older schema 1"
    ));
    assert!(stdout.contains("capability manifest,.niles/capabilities/codex.json,older schema 1"));
    assert!(!stdout.contains("global Niles index"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("doctor found non-current"));
}

#[test]
fn doctor_dirty_source_tree_never_reports_stale_no() {
    let niles = env!("CARGO_BIN_EXE_niles");
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

    let output = Command::new(niles)
        .arg("doctor")
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
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
    assert!(stdout.contains("niles 0.1.0 ("));
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
