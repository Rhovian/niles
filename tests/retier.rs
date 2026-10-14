#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;
use common::*;
use std::fs;

#[test]
fn retier_preflight_never_sends_or_records_rejected_bindings() {
    let env = TestEnv::new("retier-preflight");
    let dir = write_worker(&env.root, "worker", "session:worker", None, b"");
    let path = dir.join("meta.json");
    let mut meta: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for (current, target, error) in [
        (
            "claude:opus:high",
            "pi:tencent/hy3:low",
            "cannot change family",
        ),
        ("codex:gpt-5.5:high", "codex:gpt-5.5:low", "issues/355"),
        (
            "claude:opus:high",
            "claude:opus",
            "expected family:model:effort",
        ),
    ] {
        meta["agent"] = current.into();
        let before = serde_json::to_vec(&meta).unwrap();
        fs::write(&path, &before).unwrap();
        let output = env.run(&["retier", "worker", target]);
        assert_failure_contains("retier", &output, error);
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    assert!(!env.tmux_log.exists());
}
