#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::process::Output;

mod common;

use common::*;

fn lead(env: &TestEnv, id: &str, pane: &str) -> PathBuf {
    let dir = state_dir(&env.root).join("sessions").join(id);
    fs::create_dir_all(&dir).unwrap();
    let meta = serde_json::json!({
        "id": id, "entry_skill": "lead", "agent": "claude", "created_at": "2026-01-02T03:04:05Z",
        "workspace": env.root, "brief": dir.join("lead.md"), "lead_pane": pane,
    });
    atomic_write(dir.join("session.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
    dir.join("decisions.log")
}

fn log(env: &TestEnv, pane: &str, args: &[&str]) -> Output {
    env.niles(&env.root, args)
        .env("TMUX_PANE", pane)
        .output()
        .unwrap()
}

#[test]
fn new_state_and_manifest_stay_outside_the_repository() {
    let env = TestEnv::new("niles-global-new");
    let dir = state_dir(&env.root);
    assert_eq!(dir, state_dir(&env.root));
    assert!(!env.root.join(".niles").exists());
    write_workspace_manifest(&env.root, "claude", "codex", "claude", "claude");
    let output = env.spawn("research", &["research", "investigate"]);
    assert_command_success("spawn", &output);
    assert!(dir.join("manifest.yaml").is_file());
    assert!(dir.join("worker/research/brief.md").is_file());
    assert!(stdout_of(&output).contains(dir.to_str().unwrap()));
    assert!(!env.root.join(".niles").exists());
}

#[test]
fn project_keys_isolate_previously_colliding_workspaces() {
    let env = TestEnv::new("niles-global-keys");
    let a = env.root.join("a/b");
    let b = env.root.join("a-b");
    for workspace in [&a, &b] {
        fs::create_dir_all(workspace).unwrap();
        let output = env
            .niles(workspace, &["wait", "--timeout", "0"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(22), "{}", stderr_of(&output));
        let expected = state_dir(workspace);
        assert!(stdout_of(&output).is_empty());
        assert!(expected.join("worker/wait.lock").is_file());
        assert!(expected.join("worker/wait.running").is_file());
        assert!(!workspace.join(".niles").exists());
    }
    assert_ne!(state_dir(&a), state_dir(&b));
    fs::remove_dir_all(state_dir(&a)).unwrap();
    assert!(state_dir(&b).join("worker/wait.lock").is_file());
    assert_eq!(env.run(&["state"]).status.code(), Some(2));
}

#[test]
fn log_appends_timestamped_decisions_to_the_current_lead() {
    let env = TestEnv::new("niles-global-log");
    let old = lead(&env, "a", "%8");
    let log_path = lead(&env, "z", "%9");
    atomic_write(&log_path, "earlier decision").unwrap();
    for (id, decision) in [
        ("impl", "security commissioned: operator asked"),
        ("-", "red: fixed"),
    ] {
        let output = log(&env, "%9", &["log", id, decision]);
        assert_command_success("log", &output);
        assert!(output.stdout.is_empty());
    }
    let body = fs::read_to_string(log_path).unwrap();
    let mut lines = body.lines();
    assert_eq!(lines.next().unwrap(), "earlier decision");
    for expected in ["impl security commissioned: operator asked", "- red: fixed"] {
        let (timestamp, decision) = lines.next().unwrap().split_once(' ').unwrap();
        chrono::DateTime::parse_from_rfc3339(timestamp).unwrap();
        assert_eq!(decision, expected);
    }
    assert!(lines.next().is_none());
    assert!(!old.exists());
    assert!(!env.root.join(".niles").exists());
}

#[test]
fn log_rejects_missing_or_foreign_sessions_and_invalid_lines() {
    let env = TestEnv::new("niles-global-log-invalid");
    assert_failure_contains(
        "missing lead",
        &env.run(&["log", "-", "decision"]),
        "current lead session",
    );
    let log_path = lead(&env, "lead", "%9");
    let foreign = log(&env, "%8", &["log", "-", "decision"]);
    assert_failure_contains("foreign lead", &foreign, "current lead session");
    for args in [
        ["log", "../escape", "decision"],
        ["log", "-", "two\nlines"],
        ["log", "-", "  "],
    ] {
        let output = log(&env, "%9", &args);
        assert!(!output.status.success(), "{args:?}");
    }
    assert!(!log_path.exists());
    let target = env.root.join("target");
    atomic_write(&target, "untouched").unwrap();
    symlink(&target, &log_path).unwrap();
    let output = log(&env, "%9", &["log", "-", "decision"]);
    assert_failure_contains("log symlink", &output, "failed to append");
    assert_eq!(fs::read_to_string(target).unwrap(), "untouched");
}

#[test]
fn log_rejects_a_session_id_that_escapes_its_directory() {
    let env = TestEnv::new("niles-global-log-escape");
    let log_path = lead(&env, "lead", "%9");
    let path = log_path.parent().unwrap().join("session.json");
    edit_json(&path, |meta| {
        meta["id"] = serde_json::json!(env.root);
    });
    let output = log(&env, "%9", &["log", "-", "decision"]);
    assert_failure_contains(
        "escaping id",
        &output,
        "session id does not match directory",
    );
    assert!(!env.root.join("decisions.log").exists());
}

#[test]
fn fixture_state_paths_ignore_unregistered_ancestors_and_share_registered_homes() {
    let ancestor = temp_workspace("niles-fixture-ancestor");
    fs::create_dir_all(ancestor.join("home")).unwrap();
    fs::create_dir_all(ancestor.join("bin")).unwrap();
    atomic_write(ancestor.join("bin/tmux"), "unrelated tmux").unwrap();
    let workspace = ancestor.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    assert_eq!(
        state_dir(&workspace),
        state_dir_at(&niles_home(&workspace), &workspace)
    );
    fs::remove_dir_all(ancestor).unwrap();

    let env = TestEnv::new("niles-fixture-nested");
    let nested = env.root.join("workspace");
    fs::create_dir_all(&nested).unwrap();
    assert_eq!(state_dir(&nested), state_dir_at(&env.home, &nested));
}
