mod common;

use common::*;
use std::process::Command;

#[test]
fn doctor_reports_binary_identity_and_workspace_schema_state() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-doctor-test");
    write_workspace_manifest(&workspace, "claude", "claude", "codex", "claude", "test");

    let output = Command::new(niles)
        .arg("doctor")
        .current_dir(&workspace)
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
fn version_includes_build_identity() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let output = Command::new(niles).arg("--version").output().unwrap();

    assert_command_success("--version", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("niles 0.1.0 ("));
    assert!(stdout.contains("built "));
}
