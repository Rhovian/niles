#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;
use common::*;
use std::fs;

#[test]
fn retier_preflight_never_sends_or_records_rejected_bindings() {
    let env = TestEnv::new("retier-preflight");
    let dir = write_worker(&env.root, "worker", "session:worker", None, b"");
    let path = dir.join("meta.json");
    let manifest = include_str!("../examples/claude-lead.yaml").replacen(
        "primary: codex:gpt-6.1-sol:medium",
        "primary: claude:sonnet:high",
        1,
    );
    fs::write(state_dir(&env.root).join("manifest.yaml"), manifest).unwrap();
    let mut meta: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for (current, target, error) in [
        (
            "claude:opus:high",
            "pi:tencent/hy3:low",
            "cannot change family",
        ),
        (
            "claude:opus:high",
            "claude:opus:low",
            "not allowed for role",
        ),
        (
            "codex:gpt-6.1-sol:high",
            "codex:gpt-6-astra:high",
            "worker window not live",
        ),
        (
            "claude:opus:high",
            "claude:opus",
            "expected family:model:effort",
        ),
        (
            "claude:opus:high",
            "claude:sonnet:high",
            "worker window not live",
        ),
        (
            "claude:opus:high",
            "claude:sonnet:medium",
            "worker window not live",
        ),
    ] {
        meta["agent"] = current.into();
        let before = serde_json::to_vec(&meta).unwrap();
        fs::write(&path, &before).unwrap();
        let output = env.run(&["retier", "worker", target]);
        assert_failure_contains("retier", &output, error);
        assert_eq!(fs::read(&path).unwrap(), before);
        let history = fs::read_to_string(dir.join("retier.log")).unwrap();
        let entry: serde_json::Value =
            serde_json::from_str(history.lines().last().unwrap()).unwrap();
        assert_eq!(entry["worker"], "worker");
        assert_eq!(entry["from"], current);
        assert_eq!(entry["to"], target);
        assert_eq!(entry["outcome"], "failed");
        assert!(entry["error"].as_str().unwrap().contains(error));
    }
    assert!(!env.tmux_log().contains("send-keys"));
}
