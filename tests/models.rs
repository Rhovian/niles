#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use std::{fs, process::Command};

#[test]
fn models_prints_the_effective_workspace_roster() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-models-test");
    fs::write(
        workspace.join("niles.yaml"),
        r#"
models:
  codex:
    gpt-5.7: { efforts: [low, med, xhigh] }
    gpt-5.5: { efforts: [] }
"#,
    )
    .unwrap();

    let output = Command::new(niles)
        .arg("models")
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert_command_success("models", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("models[18]{family,model,efforts}:\n"));
    assert!(stdout.contains("  codex,gpt-5.5,\n"));
    assert!(stdout.contains("  codex,gpt-5.7,low medium xhigh\n"));
}
