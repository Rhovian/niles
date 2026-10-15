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
    fs::write(env.root.join(".niles/manifest.yaml"), manifest).unwrap();
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
    }
    assert!(!env.tmux_log().contains("send-keys"));
}
