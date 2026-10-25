#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use std::fs;

const VERSION: &str = r#"if [ "$1" = --version ]; then
  if [ -f "$0.changed" ]; then printf 'Fake CLI 2.0.0\n'; else printf 'Fake CLI 1.0.0\n'; fi
  exit 0
fi
"#;
const SAMPLE_LABEL: &str = r#"primary=$HERMES_EPHEMERAL_SYSTEM_PROMPT
prompt=
for arg do
  case "$previous" in
    --append-system-prompt) primary=$arg;;
    --query-file) prompt=$(cat "$arg");;
    --config) case "$arg" in developer_instructions=*) primary=${arg#developer_instructions=}; primary=${primary#\"}; primary=${primary%\"};; esac;;
  esac
  previous=$arg
done
[ -n "$prompt" ] || prompt=$arg
sample_label=${primary##* }
[ "$primary" = "The sample label for this Niles compatibility check is $sample_label" ] || exit 4
[ "$prompt" = "For the Niles compatibility check, run \`niles agent status working <sample-label>\` using this run's sample label, then reply with only the word done." ] || exit 5
"$NILES_TEST_BINARY" agent status working "$sample_label"
"#;

fn run_fake(body: &str) -> std::process::Output {
    let env = TestEnv::new("niles-compat");
    write_executable(
        &env.bin.join("claude"),
        &format!("#!/bin/sh\n{VERSION}{body}"),
    );
    write_workspace_manifest(&env.root, "claude:haiku", "codex", "claude", "codex");
    env.niles(&env.root, &["compat", "claude"])
        .env("NILES_TEST_BINARY", env!("CARGO_BIN_EXE_niles"))
        .output()
        .unwrap()
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
fn successful_sample_label_turn() {
    let output = run_fake(SAMPLE_LABEL);
    assert_command_success("compat", &output);
    assert_eq!(
        stdout_of(&output),
        "family\tversion\tresult\tdetail\nclaude\tFake CLI 1.0.0\tpass\t\n"
    );
    assert!(stderr_of(&output).contains("'--dangerously-skip-permissions' '-p' '--model' 'haiku'"));
    assert!(!stderr_of(&output).contains("--effort"));
}

#[test]
fn missing_or_wrong_sample_label_fails() {
    for body in [
        "exit 0\n",
        "printf wrong > \"$NILES_AGENT_DIR/status.log\"\n",
    ] {
        assert_failed_turn(body, "sample label not written (tool use blocked or skipped)");
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
        &format!("{SAMPLE_LABEL}printf changed > \"$0.changed\"\n"),
        "version changed during turn: Fake CLI 1.0.0 -> Fake CLI 2.0.0",
    );
}

#[test]
fn missing_binary_fails() {
    let env = TestEnv::new("niles-compat-missing");
    write_workspace_manifest(&env.root, "claude", "codex", "claude", "codex");
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
    write_workspace_manifest(
        &env.root,
        "claude",
        "codex",
        "hermes:z-ai/glm-5.3-flash:high",
        "pi:deepseek/deepseek-v4.1-flash:high",
    );
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
            &format!("#!/bin/sh\n{VERSION}{check}{SAMPLE_LABEL}"),
        );
    }
    let output = env
        .niles(&env.root, &["compat"])
        .env("NILES_TEST_BINARY", env!("CARGO_BIN_EXE_niles"))
        .output()
        .unwrap();
    assert_command_success("compat all", &output);
    assert_eq!(stdout_of(&output).matches("\tpass\t").count(), 4);
    assert!(
        stderr_of(&output)
            .contains("'--model' 'gpt-6.1-sol' '--config' 'model_reasoning_effort=\"medium\"'")
    );
}

#[test]
fn unused_family_is_skipped_and_missing_manifest_errors() {
    let env = TestEnv::new("niles-compat-selection");
    assert_failure_contains(
        "no manifest",
        &env.run(&["compat"]),
        "compat requires a workspace manifest",
    );
    write_workspace_manifest(&env.root, "claude", "codex", "claude", "codex");
    let output = env.run(&["compat", "hermes"]);
    assert_command_success("unused family", &output);
    assert!(stdout_of(&output).contains("hermes\t-\tskip\tnot used by manifest"));
}
