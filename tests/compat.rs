#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use std::fs;

const VERSION: &str = r#"if [ "$1" = --version ]; then
  if [ -f "$0.changed" ]; then printf 'Fake CLI 2.0.0\n'; else printf 'Fake CLI 1.0.0\n'; fi
  exit 0
fi
"#;
const NONCE: &str = r#"for prompt do :; done
case "$prompt" in */prompt.md) prompt=$(cat "$prompt");; esac
nonce=${prompt#*printf %s }
nonce=${nonce%% *}
printf %s "$nonce" > niles-compat-nonce
"#;

fn run_fake(body: &str) -> std::process::Output {
    let env = TestEnv::new("niles-compat");
    write_executable(
        &env.bin.join("claude"),
        &format!("#!/bin/sh\n{VERSION}{body}"),
    );
    // The shipped profile must win over workspace overrides.
    fs::write(
        env.root.join("niles.yaml"),
        "agents:\n  claude:\n    binary: missing-cli\n",
    )
    .unwrap();
    env.run(&["compat", "claude"])
}

fn assert_failed_turn(body: &str, detail: &str) {
    let output = run_fake(body);
    assert_eq!(output.status.code(), Some(1));
    let stdout = stdout_of(&output);
    assert!(stdout.contains(&format!("\tfail\t{detail}")), "{stdout}");
    let dir = stdout.trim_end().rsplit_once("; evidence: ").unwrap().1;
    assert!(std::path::Path::new(dir).join("stderr").exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn successful_nonce_turn() {
    let output = run_fake(NONCE);
    assert_command_success("compat", &output);
    assert_eq!(
        stdout_of(&output),
        "family\tversion\tresult\tdetail\nclaude\tFake CLI 1.0.0\tpass\t\n"
    );
    assert!(
        stderr_of(&output)
            .contains("'--dangerously-skip-permissions' '-p' '--model' 'opus' '--effort' 'low'")
    );
}

#[test]
fn missing_or_wrong_nonce_fails() {
    for body in ["exit 0\n", "printf wrong > niles-compat-nonce\n"] {
        assert_failed_turn(body, "nonce not written (tool use blocked or skipped)");
    }
}

#[test]
fn nonzero_exit_reports_last_stderr_line() {
    assert_failed_turn(
        "printf 'first\\nrejected\\n\\n' >&2\nexit 3\n",
        "exit 3: rejected",
    );
}

#[test]
fn version_change_fails() {
    assert_failed_turn(
        &format!("{NONCE}printf changed > \"$0.changed\"\n"),
        "version changed during turn: Fake CLI 1.0.0 -> Fake CLI 2.0.0",
    );
}

#[test]
fn missing_binary_fails() {
    let env = TestEnv::new("niles-compat-missing");
    fs::remove_file(env.bin.join("claude")).unwrap();
    let output = env
        .niles(&env.root, &["compat", "claude"])
        .env("PATH", &env.bin)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout_of(&output).contains("claude\tmissing\tfail\tnot found on PATH"));
}

#[test]
fn unknown_family_fails_before_running_turns() {
    let env = TestEnv::new("niles-compat-unknown");
    let output = env.run(&["compat", "claude", "custom"]);
    assert_failure_contains(
        "compat unknown",
        &output,
        "unknown built-in family `custom`",
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn all_families_use_their_brief_and_launch_environment() {
    let env = TestEnv::new("niles-compat-all");
    for family in ["claude", "codex", "hermes", "pi"] {
        let check = match family {
            "claude" => {
                "[ \"$CLAUDE_CODE_DISABLE_AGENT_VIEW\" = 1 ] && [ \"$CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION\" = false ] || exit 3\n"
            }
            "pi" => "[ \"$PI_OFFLINE\" = 1 ] || exit 3\n",
            _ => "",
        };
        write_executable(
            &env.bin.join(family),
            &format!("#!/bin/sh\n{VERSION}{check}{NONCE}"),
        );
    }
    let output = env.run(&["compat"]);
    assert_command_success("compat all", &output);
    assert_eq!(stdout_of(&output).matches("\tpass\t").count(), 4);
}
