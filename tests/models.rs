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
    - {effort: high, tier: mechanical, description: local, speed: 1, context_window: 42}
    - {effort: low, tier: standard, description: other, speed: null, context_window: 99}
"#,
    )
    .unwrap();
    fs::write(
        workspace.join("niles.yaml"),
        "models: {codex: {gpt-5.5: {efforts: [high, fresh]}, added: {efforts: [], context_window: 123}}}",
    )
    .unwrap();
    write_manifest(
        workspace,
        [
            ("lead", "claude:haiku"),
            ("worker", "codex:gpt-5.5:high"),
            ("reviewer", "codex:added"),
            ("security", "claude:haiku"),
            ("design", "claude:haiku"),
            ("research", "codex:gpt-5.5:fresh"),
        ],
    );
    let output = env.run(&["models"]);

    assert_command_success("models", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout
            .starts_with("models[4]{family,model,effort,tier,description,speed,context_window}:\n")
    );
    assert!(stdout.contains("  codex,gpt-5.5,high,mechanical,\"local\",1,42\n"));
    assert!(stdout.contains("codex,added,,unknown,unknown,unknown,123"));
    assert!(stdout.contains("codex,gpt-5.5,fresh,unknown,unknown,unknown,unknown"));
    assert!(stdout.contains("  claude,haiku,,mechanical,"));
}
