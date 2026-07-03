mod common;

use common::*;
use std::{fs, path::Path, process::Command};

#[test]
fn doctor_reports_binary_identity_and_workspace_schema_state() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-doctor-test");
    write_workspace_manifest(&workspace, "claude", "claude", "codex", "claude", "test");

    let output = Command::new(niles)
        .arg("doctor")
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
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
    assert!(stdout.contains("dev_mode: no"));
}

#[test]
fn doctor_reports_global_index_and_all_scanned_artifact_classes_nonzero() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-doctor-artifacts-test");
    let home = niles_home(&workspace);

    fs::create_dir_all(workspace.join(".niles/runs/run-1/steps")).unwrap();
    fs::create_dir_all(workspace.join(".niles/worker")).unwrap();
    fs::create_dir_all(workspace.join(".niles/sessions/session-1")).unwrap();
    fs::create_dir_all(workspace.join(".niles/capabilities")).unwrap();
    fs::create_dir_all(home.join("runs")).unwrap();
    fs::write(
        workspace.join(".niles/manifest.yaml"),
        "manager: claude\nplanner: claude\nimplementer: codex\nreviewer: claude\nvalidation_command: test\n",
    )
    .unwrap();
    fs::write(workspace.join(".niles/runs/run-1/plan.json"), "{}").unwrap();
    fs::write(
        workspace.join(".niles/runs/run-1/steps/001-step.json"),
        "{}",
    )
    .unwrap();
    fs::write(
        workspace.join(".niles/runs/run-1.json"),
        format!(
            r#"{{"id":"run-1","workspace":"{}","run_dir":"{}"}}"#,
            workspace.display(),
            workspace.join(".niles/runs/run-1").display()
        ),
    )
    .unwrap();
    fs::write(
        workspace.join(".niles/worker/worker-1.json"),
        format!(
            r#"{{"id":"worker-1","workspace":"{}","worker_dir":"{}","local_stores":[]}}"#,
            workspace.display(),
            workspace.join(".niles/worker/worker-1").display()
        ),
    )
    .unwrap();
    fs::write(
        workspace.join(".niles/sessions/session-1/session.json"),
        "{}",
    )
    .unwrap();
    fs::write(workspace.join(".niles/capabilities/codex.json"), "{}").unwrap();
    fs::write(home.join("runs/index.json"), r#"{"runs":{},"workers":{}}"#).unwrap();

    let output = Command::new(niles)
        .arg("doctor")
        .current_dir(&workspace)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("workspace manifest,.niles/manifest.yaml,older schema 1"));
    assert!(stdout.contains("run pointer,.niles/runs/run-1.json,older schema 1"));
    assert!(stdout.contains("worker pointer,.niles/worker/worker-1.json,older schema 1"));
    assert!(stdout.contains("run plan,.niles/runs/run-1/plan.json,older schema 1"));
    assert!(stdout.contains("step record,.niles/runs/run-1/steps/001-step.json,older schema 1"));
    assert!(stdout.contains(
        "manager session metadata,.niles/sessions/session-1/session.json,older schema 1"
    ));
    assert!(stdout.contains("capability manifest,.niles/capabilities/codex.json,older schema 1"));
    assert!(stdout.lines().any(|line| {
        line.contains("global Niles index,")
            && line.contains(".niles-test-home/runs/index.json")
            && line.contains("older schema 1")
    }));
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
