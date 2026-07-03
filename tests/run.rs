mod common;

use common::*;
use std::{
    fs,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

#[test]
fn run_executes_steps_and_persists_state() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-test");

    let task = write_task(
        &workspace,
        r#"
goal: "Exercise test runner"
agents:
  echo:
    binary: /bin/echo
steps:
  - agent: echo
    task: "hello test"
  - command: pwd
commands:
  pwd: pwd
"#,
    );

    let output = prepare_run(niles, &workspace, &task);
    let id = run_id(&output);
    let state_path = workspace.join(".niles/runs").join(&id).join("state.json");
    let state_body = fs::read_to_string(&state_path).unwrap();
    assert!(state_body.contains(r#""niles_schema": 2"#));
    let pointer_body =
        fs::read_to_string(workspace.join(".niles/runs").join(format!("{id}.json"))).unwrap();
    assert!(pointer_body.contains(r#""niles_schema": 2"#));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("status: created"));
    assert!(stdout.contains("next: niles step "));
    assert!(stdout.contains("exec-step: niles exec-step "));
    assert!(!stdout.contains("hello test"));

    let steps = drive_exec_steps(niles, &workspace, 1..=2);
    assert!(String::from_utf8_lossy(&steps[0].stdout).contains("hello test"));
    assert!(String::from_utf8_lossy(&steps[1].stdout).contains("status: completed"));

    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(status.status.success());

    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("status: completed"));
    assert!(status_stdout.contains("steps[2]{index,kind,label,status,exit}:"));
    assert!(status_stdout.contains("1,agent,echo,completed,0"));
    assert!(status_stdout.contains("help[4]:"));
    assert!(status_stdout.contains("Run `niles status "));
    assert!(status_stdout.contains("--json`"));

    let status_json = Command::new(niles)
        .arg("status")
        .arg("--json")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(status_json.status.success());

    let status_json_stdout = String::from_utf8_lossy(&status_json.stdout);
    assert!(status_json_stdout.contains("\"status\": \"completed\""));
    assert!(status_json_stdout.contains("001-echo.stdout.txt"));

    let show = Command::new(niles)
        .arg("show")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(show.status.success());

    let show_stdout = String::from_utf8_lossy(&show.stdout);
    assert!(show_stdout.contains("status: completed"));
    assert!(show_stdout.contains("1. agent echo completed"));
    assert!(show_stdout.contains("2. command pwd completed"));

    let log = Command::new(niles)
        .arg("log")
        .arg("--step")
        .arg("1")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(log.status.success());
    let log_stdout = String::from_utf8_lossy(&log.stdout);
    assert!(log_stdout.contains("hello test"));
    assert!(log_stdout.contains("Niles handoff context: "));

    let alias = Command::new(niles)
        .arg("l")
        .arg("--step")
        .arg("2")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(alias.status.success());
    assert!(String::from_utf8_lossy(&alias.stdout).contains("niles-test-"));
}

#[test]
fn parseable_legacy_run_state_is_stamped_on_next_state_write() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-legacy-state-stamp");

    let task = write_task(
        &workspace,
        r#"
goal: "Stamp legacy state"
steps:
  - command: ok
commands:
  ok: printf 'ok\n'
"#,
    );

    let output = prepare_run(niles, &workspace, &task);
    let id = run_id(&output);
    let state_path = workspace.join(".niles/runs").join(&id).join("state.json");
    let state_body = fs::read_to_string(&state_path).unwrap();
    let legacy_state = state_body
        .lines()
        .filter(|line| !line.contains("niles_schema"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&state_path, legacy_state).unwrap();

    let exec = exec_step_output(niles, &workspace, 1);
    assert_command_success("exec-step legacy state", &exec);
    let rewritten = fs::read_to_string(&state_path).unwrap();
    assert!(rewritten.contains(r#""niles_schema": 2"#));
    assert!(rewritten.contains(r#""status": "completed""#));
}

#[test]
fn old_run_state_reports_schema_skew_without_raw_serde_error() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-old-run-state");
    let run_dir = workspace.join(".niles/runs/old-run");
    fs::create_dir_all(&run_dir).unwrap();
    fs::write(run_dir.join("state.json"), r#"{"id":"old-run"}"#).unwrap();

    let output = Command::new(niles)
        .args(["status", "old-run"])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("run state"));
    assert!(stderr.contains("state.json"));
    assert!(stderr.contains("schema 1"));
    assert!(stderr.contains("expects 2"));
    assert!(stderr.contains("remove the run directory"));
    assert!(!stderr.contains("missing field"));

    let json_output = Command::new(niles)
        .args(["status", "old-run", "--json"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(!json_output.status.success());
    let json_stderr = String::from_utf8_lossy(&json_output.stderr);
    assert!(json_stderr.contains("run state"));
    assert!(json_stderr.contains("schema 1"));
    assert!(!json_stderr.contains("missing field"));
}

#[test]
fn run_captures_git_diff_after_each_step() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-diff-test");

    assert!(
        Command::new("git")
            .arg("init")
            .current_dir(&workspace)
            .output()
            .unwrap()
            .status
            .success()
    );
    fs::write(workspace.join("tracked.txt"), "before\n").unwrap();
    assert!(
        Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(&workspace)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        Command::new("git")
            .args([
                "-c",
                "user.name=Niles Test",
                "-c",
                "user.email=niles@example.invalid",
                "commit",
                "-m",
                "initial",
            ])
            .current_dir(&workspace)
            .output()
            .unwrap()
            .status
            .success()
    );

    let task = write_task(
        &workspace,
        r#"
goal: "Capture diff"
steps:
  - command: edit
commands:
  edit: printf 'after\n' > tracked.txt
"#,
    );

    prepare_run(niles, &workspace, &task);
    drive_exec_steps(niles, &workspace, [1]);

    let status = Command::new(niles)
        .arg("status")
        .arg("--json")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("001-edit.diff"));

    let diff = Command::new(niles)
        .arg("diff")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(diff.status.success());

    let diff_stdout = String::from_utf8_lossy(&diff.stdout);
    assert!(diff_stdout.contains("-before"));
    assert!(diff_stdout.contains("+after"));
}

#[test]
fn run_uses_project_config_defaults() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-config-test");

    fs::write(
        workspace.join("niles.yaml"),
        r#"
agents:
  echo:
    binary: /bin/echo
commands:
  marker: printf 'from config\n'
"#,
    )
    .unwrap();

    let task = write_task(
        &workspace,
        r#"
goal: "Use project config"
steps:
  - agent: echo
    task: "configured agent"
  - command: marker
"#,
    );

    prepare_run(niles, &workspace, &task);
    drive_exec_steps(niles, &workspace, 1..=2);

    let first_log = Command::new(niles)
        .arg("log")
        .arg("--step")
        .arg("1")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(first_log.status.success());
    let first_log_stdout = String::from_utf8_lossy(&first_log.stdout);
    assert!(first_log_stdout.contains("configured agent"));
    assert!(first_log_stdout.contains("Niles handoff context: "));

    let second_log = Command::new(niles)
        .arg("log")
        .arg("--step")
        .arg("2")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(second_log.status.success());
    assert_eq!(String::from_utf8_lossy(&second_log.stdout), "from config\n");
}

#[test]
fn agent_steps_map_model_effort_specs_into_invocations_and_status() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-run-tier-test");

    let codex = workspace.join("codex");
    write_executable(
        &codex,
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.142.4\n'; exit 0 ;;
esac
for arg in "$@"; do
  printf '[%s]\n' "$arg"
done
"#,
    );

    let task = write_task(
        &workspace,
        &format!(
            r#"
goal: "Run tiered codex"
agents:
  codex:
    binary: {}
steps:
  - agent: codex:gpt-5.5:xhigh
    task: "hello tier"
"#,
            codex.display()
        ),
    );

    let prepare = prepare_run(niles, &workspace, &task);
    assert!(String::from_utf8_lossy(&prepare.stdout).contains("codex:gpt-5.5:xhigh"));

    let step = exec_step_output(niles, &workspace, 1);
    assert_command_success("tiered codex exec-step", &step);
    let step_stdout = String::from_utf8_lossy(&step.stdout);
    assert!(step_stdout.contains("[exec]"));
    assert!(step_stdout.contains("[--sandbox]"));
    assert!(step_stdout.contains("[workspace-write]"));
    assert!(step_stdout.contains("[--model]"));
    assert!(step_stdout.contains("[gpt-5.5]"));
    assert!(step_stdout.contains("[--config]"));
    assert!(step_stdout.contains("[model_reasoning_effort=\"xhigh\"]"));

    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert_command_success("status", &status);
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("agent_tiers[1]{index,agent_family,model,effort}:"));
    assert!(status_stdout.contains("1,codex,gpt-5.5,xhigh"));

    let show = Command::new(niles)
        .arg("show")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert_command_success("show", &show);
    assert!(
        String::from_utf8_lossy(&show.stdout)
            .contains("agent_family codex model gpt-5.5 effort xhigh")
    );
}

#[test]
fn manifest_role_bindings_accept_model_effort_agent_specs() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-manifest-tier-test");

    let claude = workspace.join("claude");
    write_executable(
        &claude,
        r#"#!/bin/sh
case "$1" in
  --version) printf '2.1.197 (Claude Code)\n'; exit 0 ;;
esac
for arg in "$@"; do
  printf '[%s]\n' "$arg"
done
"#,
    );
    fs::write(
        workspace.join("niles.yaml"),
        format!(
            r#"
agents:
  claude:
    binary: {}
"#,
            claude.display()
        ),
    )
    .unwrap();
    fs::create_dir_all(workspace.join(".niles")).unwrap();
    fs::write(
        workspace.join(".niles/manifest.yaml"),
        r#"
manager: "claude:opus:max"
planner: "claude:sonnet:med"
implementer: "claude:sonnet:med"
reviewer: "claude:opus:max"
validation_command: "test"
"#,
    )
    .unwrap();

    let task = write_task(
        &workspace,
        r#"
goal: "Resolve tiered manifest role"
steps:
  - role: implementer
    task: "implement with sonnet"
"#,
    );

    let prepare = prepare_run(niles, &workspace, &task);
    let prepare_stdout = String::from_utf8_lossy(&prepare.stdout);
    assert!(prepare_stdout.contains("1 implementer agent claude:sonnet:med"));

    let step = exec_step_output(niles, &workspace, 1);
    assert_command_success("tiered manifest exec-step", &step);
    let step_stdout = String::from_utf8_lossy(&step.stdout);
    assert!(step_stdout.contains("[-p]"));
    assert!(step_stdout.contains("[--model]"));
    assert!(step_stdout.contains("[sonnet]"));
    assert!(step_stdout.contains("[--effort]"));
    assert!(step_stdout.contains("[medium]"));

    let status_json = Command::new(niles)
        .arg("status")
        .arg("--json")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert_command_success("status --json", &status_json);
    let status_json_stdout = String::from_utf8_lossy(&status_json.stdout);
    assert!(status_json_stdout.contains(r#""agent_family": "claude""#));
    assert!(status_json_stdout.contains(r#""model": "sonnet""#));
    assert!(status_json_stdout.contains(r#""effort": "medium""#));
}

#[test]
fn run_enforces_known_agent_cli_min_version_and_allows_override() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-version-run-test");

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_executable(
        &bin.join("codex"),
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.1.0\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );

    let task = write_task(
        &workspace,
        r#"
goal: "Gate codex"
steps:
  - agent: codex
    task: "hello"
"#,
    );

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let blocked = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(!blocked.status.success());
    let blocked_stderr = String::from_utf8_lossy(&blocked.stderr);
    assert!(blocked_stderr.contains("codex CLI 0.1.0 is below the supported minimum"));
    assert!(blocked_stderr.contains("--allow-cli-mismatch"));

    let allowed = Command::new(niles)
        .args(["run", "--allow-cli-mismatch"])
        .arg(&task)
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", niles_home(&workspace))
        .output()
        .unwrap();
    assert_command_success("run --allow-cli-mismatch", &allowed);
    assert!(String::from_utf8_lossy(&allowed.stdout).contains("status: created"));
    assert!(String::from_utf8_lossy(&allowed.stderr).contains("CLI mismatch override is enabled"));
}

#[test]
fn analyze_reports_version_gate_status() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-version-analyze-test");

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_executable(
        &bin.join("codex"),
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.1.0\n'; exit 0 ;;
  --help) printf 'codex help\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let analyze = Command::new(niles)
        .args(["analyze", "--agent", "codex"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_command_success("analyze", &analyze);

    let stdout = String::from_utf8_lossy(&analyze.stdout);
    assert!(stdout.contains("version_gate: codex fail 0.1.0"));
    assert!(stdout.contains("wrote .niles/capabilities/codex.json"));

    let manifest = fs::read_to_string(workspace.join(".niles/capabilities/codex.json")).unwrap();
    assert!(manifest.contains(r#""version_gate""#));
    assert!(manifest.contains(r#""status": "fail""#));
}

#[test]
fn bare_niles_errors_when_stdin_is_not_interactive() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-session-noninteractive-test");
    write_workspace_manifest(&workspace, "claude", "claude", "codex", "claude", "test");

    let output = Command::new(niles)
        .args(["--goal", "Fix the startup flow"])
        .current_dir(&workspace)
        .env("TMUX", "/tmp/niles-test-tmux")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(workspace.join(".niles/worker").is_dir());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("stdin is not interactive"));
    assert!(stderr.contains("choose the manager agent"));
}

#[test]
fn agent_steps_receive_context_artifacts() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-context-test");

    fs::write(
        workspace.join("agent.sh"),
        r#"#!/bin/sh
printf 'PROMPT\n%s\n' "$1"
context=$(printf '%s\n' "$1" | sed -n 's/^Niles handoff context: //p' | head -n 1)
if [ -n "$context" ]; then
  printf 'CONTEXT\n'
  cat "$context"
fi
"#,
    )
    .unwrap();

    let task = write_task(
        &workspace,
        r#"
goal: "Use context handoffs"
agents:
  inspector:
    binary: sh
    args: ["agent.sh"]
steps:
  - agent: inspector
    role: planner
    task: "planner says inspect auth flow"
  - agent: inspector
    role: implementer
    task: "implement from planner output"
  - command: validate
    role: validation
  - agent: inspector
    role: reviewer
    task: "review with validation output"
commands:
  validate: printf 'validation ok\n'
"#,
    );

    prepare_run(niles, &workspace, &task);
    let steps = drive_exec_steps(niles, &workspace, 1..=4);
    let run_stdout = steps
        .iter()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(run_stdout.contains(".niles/runs/"));

    let implementer_log = Command::new(niles)
        .arg("log")
        .arg("--step")
        .arg("2")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(implementer_log.status.success());
    let implementer_stdout = String::from_utf8_lossy(&implementer_log.stdout);
    assert!(implementer_stdout.contains("Niles handoff context: "));
    assert!(implementer_stdout.contains("# Niles Step Context"));
    assert!(implementer_stdout.contains("role: implementer"));
    assert!(implementer_stdout.contains("## Prior Agent Output"));
    assert!(implementer_stdout.contains("planner says inspect auth flow"));

    let reviewer_log = Command::new(niles)
        .arg("log")
        .arg("--step")
        .arg("4")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(reviewer_log.status.success());
    let reviewer_stdout = String::from_utf8_lossy(&reviewer_log.stdout);
    assert!(reviewer_stdout.contains("role: reviewer"));
    assert!(reviewer_stdout.contains("## Validation Output"));
    assert!(reviewer_stdout.contains("validation ok"));
    assert!(reviewer_stdout.contains("## Latest Diff"));

    let show = Command::new(niles)
        .arg("show")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(show.status.success());
    assert!(String::from_utf8_lossy(&show.stdout).contains("context "));
    assert!(String::from_utf8_lossy(&show.stdout).contains(".niles/runs/"));

    let status_json = Command::new(niles)
        .arg("status")
        .arg("--json")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(status_json.status.success());
    assert!(String::from_utf8_lossy(&status_json.stdout).contains("\"context\": "));
    assert!(String::from_utf8_lossy(&status_json.stdout).contains(".niles/runs/"));
}

#[test]
fn run_prints_actionable_failure_summary() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-failure-test");

    let task = write_task(
        &workspace,
        r#"
goal: "Fail usefully"
steps:
  - command: fail
commands:
  fail:
    run: "for i in 1 2 3 4 5 6 7 8 9 10 11 12 13; do echo tail-line-$i >&2; done; exit 7"
"#,
    );

    prepare_run(niles, &workspace, &task);
    let output = exec_step_output(niles, &workspace, 1);

    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("status: failed"));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failure:"));
    assert!(stderr.contains("step: 1 command fail"));
    assert!(stderr.contains("exit: 7"));
    assert!(stderr.contains("stderr: "));
    assert!(stderr.contains("diff: "));
    assert!(stderr.contains(".niles/runs/"));
    assert!(stderr.contains("stderr tail:"));
    assert!(stderr.contains("  tail-line-2"));
    assert!(stderr.contains("  tail-line-13"));

    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(status.status.success());

    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("status: failed"));
    assert!(status_stdout.contains("1,command,fail,failed,7"));
    assert!(status_stdout.contains("--stderr`"));
}

#[test]
fn exec_step_error_appends_failed_backstop() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-exec-error-backstop");

    let task = write_task(
        &workspace,
        r#"
goal: "Exec error backstop"
steps:
  - command: missing
"#,
    );

    let prepare = prepare_run(niles, &workspace, &task);
    let id = run_id(&prepare);
    let output = exec_step_output(niles, &workspace, 1);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown command `missing`"));

    let status_log =
        fs::read_to_string(workspace.join(".niles/runs").join(&id).join("status.log")).unwrap();
    assert!(status_log.contains("failed: step 1 exec error: unknown command `missing`"));

    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert_command_success("status", &status);
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("status: failed"));
    assert!(status_stdout.contains("1,command,missing,failed,-"));
}

#[test]
fn resume_continues_from_first_incomplete_step() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-resume-test");

    let task = write_task(
        &workspace,
        r#"
goal: "Resume failed run"
steps:
  - command: first
  - command: gate
  - command: last
commands:
  first: printf 'first\n' >> trace.txt
  gate: test -f allow
  last: printf 'last\n' >> trace.txt
"#,
    );

    prepare_run(niles, &workspace, &task);
    let first = exec_step_output(niles, &workspace, 1);
    assert_command_success("exec-step 1", &first);

    let failed = exec_step_output(niles, &workspace, 2);
    assert!(
        !failed.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&failed.stdout),
        String::from_utf8_lossy(&failed.stderr)
    );
    assert!(String::from_utf8_lossy(&failed.stdout).contains("status: failed"));

    fs::write(workspace.join("allow"), "").unwrap();

    let resumed = Command::new(niles)
        .arg("resume")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(
        resumed.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&resumed.stdout),
        String::from_utf8_lossy(&resumed.stderr)
    );

    let resumed_stdout = String::from_utf8_lossy(&resumed.stdout);
    assert!(resumed_stdout.contains("resume: "));
    assert!(resumed_stdout.contains("from_step: 2"));
    assert!(resumed_stdout.contains("next: niles exec-step "));
    assert!(resumed_stdout.contains("exec-step: niles exec-step "));
    assert!(resumed_stdout.contains("wait: niles wait "));

    drive_exec_steps(niles, &workspace, 2..=3);

    assert_eq!(
        fs::read_to_string(workspace.join("trace.txt")).unwrap(),
        "first\nlast\n"
    );

    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(status.status.success());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("status: completed"));
    assert!(status_stdout.contains("1,command,first,completed,0"));
    assert!(status_stdout.contains("2,command,gate,completed,0"));
    assert!(status_stdout.contains("3,command,last,completed,0"));
}

#[test]
fn manifest_command_is_removed() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-manifest-removed-test");

    let output = Command::new(niles)
        .args(["manifest", "Ship", "workflow"])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unrecognized subcommand"));
    assert!(stderr.contains("manifest"));
}

#[test]
fn status_shows_running_and_pending_steps() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-running-test");

    let task = write_task(
        &workspace,
        r#"
goal: "Show running state"
steps:
  - command: slow
    role: validation
  - command: fast
    role: validation
commands:
  slow: sleep 1
  fast: printf 'done\n'
"#,
    );

    prepare_run(niles, &workspace, &task);

    let child = Command::new(niles)
        .args(["exec-step", "latest", "1"])
        .current_dir(&workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut saw_running = false;
    for _ in 0..20 {
        let status = Command::new(niles)
            .arg("status")
            .current_dir(&workspace)
            .output();

        if let Ok(status) = status
            && status.status.success()
        {
            let stdout = String::from_utf8_lossy(&status.stdout);
            if stdout.contains("1,validation,command,slow,running,-")
                && stdout.contains("2,validation,command,fast,pending,-")
            {
                saw_running = true;
                break;
            }
        }

        thread::sleep(Duration::from_millis(100));
    }

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(saw_running, "status never showed running/pending steps");
}

#[test]
fn watch_streams_run_state_until_completion() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-watch-test");

    let task = write_task(
        &workspace,
        r#"
goal: "Watch running state"
steps:
  - command: slow
    role: validation
  - command: fast
    role: validation
commands:
  slow: sleep 3
  fast: printf 'done\n'
"#,
    );

    prepare_run(niles, &workspace, &task);

    let step1 = Command::new(niles)
        .args(["exec-step", "latest", "1"])
        .current_dir(&workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut run_started = false;
    for _ in 0..30 {
        let status = Command::new(niles)
            .arg("status")
            .current_dir(&workspace)
            .output();

        if let Ok(status) = status
            && status.status.success()
        {
            let stdout = String::from_utf8_lossy(&status.stdout);
            if stdout.contains("1,validation,command,slow,running,-")
                && stdout.contains("2,validation,command,fast,pending,-")
            {
                run_started = true;
                break;
            }
        }

        thread::sleep(Duration::from_millis(100));
    }
    assert!(run_started, "run never reached running state");

    let watch = Command::new(niles)
        .args(["watch", "--interval", "0.05", "--no-clear"])
        .current_dir(&workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let output = step1.wait_with_output().unwrap();
    assert_command_success("exec-step 1", &output);

    let step2 = exec_step_output(niles, &workspace, 2);
    assert_command_success("exec-step 2", &step2);

    let watch = watch.wait_with_output().unwrap();
    assert_command_success("watch", &watch);

    let stdout = String::from_utf8_lossy(&watch.stdout);
    assert!(stdout.contains("status: running"));
    assert!(stdout.contains("1,validation,command,slow,running,-"));
    assert!(stdout.contains("2,validation,command,fast,pending,-"));
    assert!(stdout.contains("status: completed"));
    assert!(stdout.contains("2,validation,command,fast,completed,0"));
}
