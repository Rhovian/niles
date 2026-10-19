#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::{fs, os::unix::fs::symlink, path::PathBuf};

use common::*;

fn lead(env: &TestEnv, id: &str, pane: &str) -> PathBuf {
    let dir = state_dir(&env.root).join("sessions").join(id);
    fs::create_dir_all(&dir).unwrap();
    let meta = serde_json::json!({
        "id": id, "agent": "claude", "created_at": "2026-01-02T03:04:05Z",
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
    let output = env.run(&["spawn", "research", "--role", "research", "investigate"]);
    assert_command_success("spawn", &output);
    assert!(dir.join("manifest.yaml").is_file());
    assert!(dir.join("worker/research/brief.md").is_file());
    assert!(stdout_of(&output).contains(dir.to_str().unwrap()));
    assert!(!env.root.join(".niles").exists());
}

#[test]
fn migration_moves_the_entire_directory_and_is_idempotent() {
    let env = TestEnv::new("niles-global-move");
    let legacy = env.root.join(".niles");
    fs::create_dir_all(legacy.join("worker/old")).unwrap();
    fs::write(legacy.join("worker/old/report.md"), b"report\0bytes").unwrap();
    fs::write(
        legacy.join("manifest.yaml"),
        include_str!("../examples/claude-lead.yaml"),
    )
    .unwrap();
    let output = env.run(&["doctor"]);
    assert_command_success("migrated manifest", &output);
    let dir = state_dir(&env.root);
    assert!(!legacy.exists());
    assert_eq!(
        fs::read(dir.join("worker/old/report.md")).unwrap(),
        b"report\0bytes"
    );
    assert!(dir.join("manifest.yaml").is_file());
    assert_eq!(state_dir(&env.root), dir);
}

#[test]
fn migration_refuses_conflicting_state_without_losing_either_copy() {
    let env = TestEnv::new("niles-global-conflict");
    let legacy = env.root.join(".niles");
    let global = state_dir(&env.root);
    for dir in [&legacy, &global] {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("manifest.yaml"), dir.to_str().unwrap()).unwrap();
    }
    assert_failure_contains(
        "conflict",
        &env.run(&["workers"]),
        "reconcile the directories",
    );
    for dir in [&legacy, &global] {
        assert_eq!(
            fs::read_to_string(dir.join("manifest.yaml")).unwrap(),
            dir.to_str().unwrap()
        );
    }
}

#[test]
fn migration_rejects_a_repository_supplied_state_symlink() {
    let env = TestEnv::new("niles-global-symlink");
    let global = state_dir(&env.root);
    let target = env.root.join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("secret"), "untouched").unwrap();
    symlink(&target, env.root.join(".niles")).unwrap();
    assert_failure_contains("symlink", &env.run(&["workers"]), "not a file or symlink");
    assert_eq!(
        fs::read_to_string(target.join("secret")).unwrap(),
        "untouched"
    );
    assert!(!global.exists());
}

#[test]
fn project_keys_use_plain_slash_replacement_and_accept_collisions() {
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
        let expected = env
            .home
            .join(".niles/state")
            .join(workspace.to_str().unwrap().replace('/', "-"));
        assert!(expected.join("worker/wait.lock").is_file());
    }
    assert_eq!(state_dir(&a), state_dir(&b));
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
fn migrated_pi_transcripts_remain_readable() {
    let env = TestEnv::new("niles-global-pi");
    let worker = write_worker(&env.root, "pi", "niles-test-session:pi", None, b"");
    let legacy = env.root.join(".niles");
    let transcript = legacy.join("worker/pi/pi-sessions");
    let path = worker.join("meta.json");
    let mut meta: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    meta["session_link"] = serde_json::json!({ "family": "pi", "session_dir": transcript });
    meta["brief"] = serde_json::json!(legacy.join("worker/pi/brief.md"));
    fs::write(path, serde_json::to_vec(&meta).unwrap()).unwrap();
    fs::rename(state_dir(&env.root), &legacy).unwrap();
    fs::create_dir_all(&transcript).unwrap();
    fs::write(transcript.join("session.jsonl"), concat!(r#"{"type":"message","timestamp":"2026-10-01T16:00:00Z","message":{"role":"assistant","stopReason":"stop","usage":{"input":20,"output":30,"cacheRead":40,"cacheWrite":50}}}"#, "\n")).unwrap();
    let output = env.run(&["usage"]);
    assert_command_success("migrated usage", &output);
    let usage: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(usage["sessions"][0]["usage"]["input_tokens"], 20);
    assert!(!legacy.exists());
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
