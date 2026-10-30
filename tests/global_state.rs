#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::{fs, os::unix::fs::symlink, path::PathBuf};

use common::*;

fn lead(env: &TestEnv, id: &str, pane: &str) -> PathBuf {
    let dir = state_dir(&env.root).join("sessions").join(id);
    fs::create_dir_all(&dir).unwrap();
    let meta = serde_json::json!({
        "id": id, "entry_skill": "lead", "agent": "claude", "created_at": "2026-01-02T03:04:05Z",
        "workspace": env.root, "brief": dir.join("lead.md"), "lead_pane": pane,
    });
    fs::write(dir.join("session.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
    dir.join("decisions.log")
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
        assert!(expected.join("worker/wait.lock").is_file());
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
    let log = lead(&env, "z", "%9");
    fs::write(&log, "earlier decision").unwrap();
    for (id, decision) in [
        ("impl", "security commissioned: operator asked"),
        ("-", "red: fixed"),
    ] {
        let output = env
            .niles(&env.root, &["log", id, decision])
            .env("TMUX_PANE", "%9")
            .output()
            .unwrap();
        assert_command_success("log", &output);
        assert!(output.stdout.is_empty());
    }
    let body = fs::read_to_string(log).unwrap();
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
    let log = lead(&env, "lead", "%9");
    let foreign = env
        .niles(&env.root, &["log", "-", "decision"])
        .env("TMUX_PANE", "%8")
        .output()
        .unwrap();
    assert_failure_contains("foreign lead", &foreign, "current lead session");
    for args in [
        ["log", "../escape", "decision"],
        ["log", "-", "two\nlines"],
        ["log", "-", "  "],
    ] {
        let output = env
            .niles(&env.root, &args)
            .env("TMUX_PANE", "%9")
            .output()
            .unwrap();
        assert!(!output.status.success(), "{args:?}");
    }
    assert!(!log.exists());
    let target = env.root.join("target");
    fs::write(&target, "untouched").unwrap();
    symlink(&target, &log).unwrap();
    let output = env
        .niles(&env.root, &["log", "-", "decision"])
        .env("TMUX_PANE", "%9")
        .output()
        .unwrap();
    assert_failure_contains("log symlink", &output, "failed to append");
    assert_eq!(fs::read_to_string(target).unwrap(), "untouched");
}

#[test]
fn log_rejects_a_session_id_that_escapes_its_directory() {
    let env = TestEnv::new("niles-global-log-escape");
    let log = lead(&env, "lead", "%9");
    let path = log.parent().unwrap().join("session.json");
    let mut meta: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    meta["id"] = serde_json::json!(env.root);
    fs::write(path, serde_json::to_vec(&meta).unwrap()).unwrap();
    let output = env
        .niles(&env.root, &["log", "-", "decision"])
        .env("TMUX_PANE", "%9")
        .output()
        .unwrap();
    assert_failure_contains(
        "escaping id",
        &output,
        "session id does not match directory",
    );
    assert!(!env.root.join("decisions.log").exists());
}

#[test]
fn workspace_wait_locks_stay_in_global_project_state() {
    let env = TestEnv::new("niles-global-wait");
    let output = env.run(&["wait", "--timeout", "0"]);
    assert_eq!(output.status.code(), Some(22), "{}", stderr_of(&output));
    assert!(stdout_of(&output).is_empty());
    let workers = state_dir(&env.root).join("worker");
    assert!(workers.join("wait.lock").is_file());
    assert!(workers.join("wait.running").is_file());
    assert!(!env.root.join(".niles").exists());
}

#[test]
fn fixture_state_paths_ignore_unregistered_ancestors_and_share_registered_homes() {
    let ancestor = temp_workspace("niles-fixture-ancestor");
    fs::create_dir_all(ancestor.join("home")).unwrap();
    fs::create_dir_all(ancestor.join("bin")).unwrap();
    fs::write(ancestor.join("bin/tmux"), "unrelated tmux").unwrap();
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
