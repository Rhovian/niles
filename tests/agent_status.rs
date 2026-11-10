#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
use common::*;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Output;

mod common;

fn status(env: &TestEnv, dir: &Path, kind: &str, detail: &[&str]) -> Output {
    let mut command = env.niles(&env.root, &["agent", "status", kind]);
    command
        .env("NILES_AGENT_DIR", dir)
        .args(detail)
        .output()
        .unwrap()
}

#[test]
fn kinds_details_reports_and_partial_lines() {
    let env = TestEnv::new("niles-agent-status");
    let dir = write_worker(&env.root, "test", "@1", None, b"");
    let log = dir.join("status.log");
    atomic_write(&log, "working: partial").unwrap();
    for kind in ["working", "blocked", "needs-decision", "failed"] {
        assert_command_success(kind, &status(&env, &dir, kind, &["  first", "second  "]));
    }
    for report in [None, Some("")] {
        if let Some(body) = report {
            atomic_write(dir.join("report.md"), body).unwrap();
        }
        let before = fs::read(&log).unwrap();
        assert_failure_contains(
            "done without report",
            &status(&env, &dir, "done", &["ready"]),
            dir.join("report.md").to_str().unwrap(),
        );
        assert_eq!(fs::read(&log).unwrap(), before);
    }
    atomic_write(dir.join("report.md"), "deliverable").unwrap();
    assert_command_success("done", &status(&env, &dir, "done", &["ready"]));
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        format!(
            "working: partial\nworking: first second\nblocked: first second\nneeds-decision: first second\nfailed: first second\ndone: ready; report: {}/report.md\n",
            dir.display()
        )
    );
    assert!(!dir.join("stage.lock").exists());
}

#[test]
fn hyphen_leading_details_are_values() {
    let env = TestEnv::new("niles-agent-hyphens");
    let log = env.root.join("status.log");
    atomic_write(&log, "").unwrap();
    for (kind, words) in [
        ("blocked", ["-x", "invalid"]),
        ("working", ["-1", "failures"]),
        ("failed", ["--foo", "missing"]),
    ] {
        assert_command_success(kind, &status(&env, &env.root, kind, &words));
    }
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        "blocked: -x invalid\nworking: -1 failures\nfailed: --foo missing\n"
    );
}

#[test]
fn invalid_details_and_niles_owned_kinds_leave_log_unchanged() {
    let env = TestEnv::new("niles-agent-invalid");
    let log = env.root.join("status.log");
    atomic_write(&log, "earlier\n").unwrap();
    for detail in [
        "",
        "   ",
        "ready\nfailed: forged",
        "ready\rforged",
        "\x1b",
        "\t",
        "\u{85}",
    ] {
        assert!(
            !status(&env, &env.root, "working", &[detail])
                .status
                .success()
        );
        assert_eq!(fs::read_to_string(&log).unwrap(), "earlier\n");
    }
    for kind in ["closed", "gone", "finished", "Done", "done:"] {
        assert_failure_contains(
            kind,
            &status(&env, &env.root, kind, &["forged"]),
            "invalid value",
        );
        assert_eq!(fs::read_to_string(&log).unwrap(), "earlier\n");
    }
}

#[test]
fn requires_worker_environment_and_existing_log_and_rejects_symlinks() {
    let env = TestEnv::new("niles-agent-env");
    let args = ["agent", "status", "working", "ready"];
    let output = env
        .niles(&env.root, &args)
        .env_remove("NILES_AGENT_DIR")
        .output()
        .unwrap();
    assert_failure_contains("unset", &output, "NILES_AGENT_DIR");
    atomic_write(env.root.join("file"), "untouched").unwrap();
    for dir in [
        PathBuf::from("relative"),
        env.root.join("absent"),
        env.root.join("file"),
        env.root.clone(),
    ] {
        let output = status(&env, &dir, "working", &["ready"]);
        assert!(!output.status.success());
        assert!(!env.root.join("status.log").exists());
    }
    atomic_write(env.root.join("status.log"), "earlier\n").unwrap();
    let output = status(&env, Path::new("."), "working", &["ready"]);
    assert_failure_contains(
        "existing relative directory",
        &output,
        "absolute worker directory",
    );
    assert_eq!(
        fs::read_to_string(env.root.join("status.log")).unwrap(),
        "earlier\n"
    );
    fs::remove_file(env.root.join("status.log")).unwrap();
    symlink(env.root.join("file"), env.root.join("status.log")).unwrap();
    assert!(
        !status(&env, &env.root, "working", &["ready"])
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(env.root.join("file")).unwrap(),
        "untouched"
    );
}

#[test]
fn help_carries_kinds_and_protocol_without_worker_environment() {
    let env = TestEnv::new("niles-agent-help");
    let output = env
        .niles(&env.root, &["agent", "status", "--help"])
        .env_remove("NILES_AGENT_DIR")
        .output()
        .unwrap();
    assert_command_success("help", &output);
    let help = stdout_of(&output);
    for text in [
        "done",
        "failed",
        "blocked",
        "needs-decision",
        "working",
        "Only a status line wakes",
        "working wakes nobody",
        "hand back, not exit",
        "lead closes you",
        "non-empty",
        "NILES_AGENT_DIR",
        "binding for every spawned worker",
        "never write status.log directly",
        "Invalid submissions fail before appending",
        "Finding tags, roles and routing are skill conventions",
    ] {
        assert!(help.contains(text), "{help}");
    }
}
