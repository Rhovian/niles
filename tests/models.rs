#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use std::fs;

#[test]
fn models_prints_the_effective_workspace_roster() {
    let env = TestEnv::new("niles-models-test");
    let workspace = &env.root;
    let home = &env.home;
    fs::create_dir_all(home.join(".niles")).unwrap();
    fs::write(
        home.join(".niles/roster.yaml"),
        r#"codex:
  gpt-5.5:
    - {effort: high, tier: mechanical, description: local, cost: null, speed: 1, context_window: 42}
    - {effort: low, tier: standard, description: other, cost: 2, speed: null, context_window: 99}
"#,
    )
    .unwrap();
    fs::write(
        workspace.join("niles.yaml"),
        "models: {codex: {gpt-5.5: {efforts: [high]}}}",
    )
    .unwrap();
    write_manifest(
        workspace,
        [
            ("lead", "claude:haiku"),
            ("worker", "codex:gpt-5.5:high"),
            ("reviewer", "codex:gpt-6-sol:high"),
            ("security", "claude:haiku"),
            ("design", "claude:haiku"),
            ("research", "claude:haiku"),
        ],
    );
    let output = env.run(&["models"]);

    assert_command_success("models", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with(
        "models[3]{family,model,effort,tier,description,cost,speed,context_window}:\n"
    ));
    assert!(stdout.contains("  codex,gpt-5.5,high,mechanical,\"local\",unknown,1,42\n"));
    assert!(stdout.contains("  codex,gpt-6-sol,high,standard,"));
    assert!(stdout.contains("  claude,haiku,,mechanical,"));
    assert!(!stdout.contains("gpt-5.5,low,"));
}
