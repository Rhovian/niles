mod common;

use common::*;
use std::{
    fs,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn prepare_then_exec_step_drives_run() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-step-test");

    let task = write_task(
        &workspace,
        r#"
goal: "Exercise manager stepping"
agents:
  echo:
    binary: /bin/echo
steps:
  - agent: echo
    task: "stepped hello"
  - command: pwd
commands:
  pwd: pwd
"#,
    );

    // prepare: create the run without executing it.
    let prepare = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
        .output()
        .unwrap();
    assert!(
        prepare.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&prepare.stdout),
        String::from_utf8_lossy(&prepare.stderr)
    );
    let prepare_stdout = String::from_utf8_lossy(&prepare.stdout);
    assert!(prepare_stdout.contains("status: created"));
    assert!(prepare_stdout.contains("1 agent echo"));
    assert!(prepare_stdout.contains("next: niles step "));
    // prepare must not execute the agent.
    assert!(!prepare_stdout.contains("stepped hello"));

    // status shows created with all steps pending.
    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("status: created"));
    assert!(status_stdout.contains("1,agent,echo,pending,-"));

    // exec-step 1: runs the agent, records state, appends a done wake.
    let step1 = Command::new(niles)
        .arg("exec-step")
        .arg("latest")
        .arg("1")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(
        step1.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&step1.stdout),
        String::from_utf8_lossy(&step1.stderr)
    );
    let step1_stdout = String::from_utf8_lossy(&step1.stdout);
    assert!(step1_stdout.contains("stepped hello"));
    assert!(step1_stdout.contains("step 1: completed"));
    // run is still running with step 2 pending; not yet complete.
    assert!(!step1_stdout.contains("status: completed"));

    // exec-step 2: runs the command and completes the run.
    let step2 = Command::new(niles)
        .arg("exec-step")
        .arg("latest")
        .arg("2")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(
        step2.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&step2.stdout),
        String::from_utf8_lossy(&step2.stderr)
    );
    assert!(String::from_utf8_lossy(&step2.stdout).contains("status: completed"));

    // the run status log carries the wake lines `niles wait` reads.
    let runs_dir = workspace.join(".niles").join("runs");
    let run_dir = fs::read_dir(&runs_dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .max()
        .unwrap();
    let status_log = fs::read_to_string(run_dir.join("status.log")).unwrap();
    assert!(status_log.contains("done: step 1 "));
    assert!(status_log.contains("done: step 2 "));

    // final status is completed with both steps recorded.
    let final_status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    let final_stdout = String::from_utf8_lossy(&final_status.stdout);
    assert!(final_stdout.contains("status: completed"));
    assert!(final_stdout.contains("1,agent,echo,completed,0"));
    assert!(final_stdout.contains("2,command,pwd,completed,0"));
}

#[test]
fn run_prepared_from_foreign_cwd_uses_workspace_store() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let root = temp_workspace("niles-foreign-cwd");
    let invocation = root.join("invocation");
    let workspace = root.join("workspace");
    let other = root.join("other");
    let home = root.join("home");
    fs::create_dir_all(&invocation).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&other).unwrap();

    let task = invocation.join("task.yaml");
    fs::write(
        &task,
        format!(
            r#"
goal: "Drive from a foreign cwd"
workspace: "{}"
agents:
  echo:
    binary: /bin/echo
steps:
  - agent: echo
    task: "foreign hello"
  - command: pwd
commands:
  pwd: pwd
"#,
            workspace.display()
        ),
    )
    .unwrap();

    let prepare = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&invocation)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("run", &prepare);
    let id = run_id(&prepare);
    let workspace_run_dir = workspace.join(".niles/runs").join(&id);
    let invocation_run_dir = invocation.join(".niles/runs").join(&id);

    assert!(workspace_run_dir.join("state.json").exists());
    assert!(workspace_run_dir.join("plan.json").exists());
    assert!(!invocation_run_dir.join("state.json").exists());

    let status = Command::new(niles)
        .args(["status", &id])
        .current_dir(&invocation)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("status from invocation cwd", &status);
    assert!(String::from_utf8_lossy(&status.stdout).contains("status: created"));

    let status_from_other = Command::new(niles)
        .args(["status", &id])
        .current_dir(&other)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("status from unrelated cwd", &status_from_other);
    assert!(String::from_utf8_lossy(&status_from_other.stdout).contains("status: created"));

    let step1 = Command::new(niles)
        .args(["exec-step", &id, "1"])
        .current_dir(&invocation)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("exec-step 1 from invocation cwd", &step1);
    assert!(String::from_utf8_lossy(&step1.stdout).contains("foreign hello"));

    let status_log = fs::read_to_string(workspace_run_dir.join("status.log")).unwrap();
    assert!(status_log.contains("done: step 1 "));
    assert!(!invocation_run_dir.join("status.log").exists());

    let wait = Command::new(niles)
        .args([
            "wait",
            &id,
            "--index",
            "1",
            "--interval",
            "0.05",
            "--timeout",
            "0",
        ])
        .current_dir(&other)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("wait from unrelated cwd", &wait);
    assert!(String::from_utf8_lossy(&wait.stdout).contains("done: step 1 "));

    let step2 = Command::new(niles)
        .args(["exec-step", &id, "2"])
        .current_dir(&other)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("exec-step 2 from unrelated cwd", &step2);
    let step2_stdout = String::from_utf8_lossy(&step2.stdout);
    assert!(step2_stdout.contains(&workspace.display().to_string()));
    assert!(step2_stdout.contains("status: completed"));

    let log = Command::new(niles)
        .args(["log", &id, "--step", "1"])
        .current_dir(&other)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("log from unrelated cwd", &log);
    assert!(String::from_utf8_lossy(&log.stdout).contains("foreign hello"));
}

#[test]
fn step_launch_from_foreign_cwd_uses_workspace_brief_and_status_log() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let root = temp_workspace("niles-foreign-step");
    let invocation = root.join("invocation");
    let workspace = root.join("workspace");
    let home = root.join("home");
    let bin = root.join("bin");
    fs::create_dir_all(&invocation).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&bin).unwrap();

    let tmux_log = root.join("tmux.log");
    write_executable(
        &bin.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  has-session) exit 1 ;;
  list-windows) exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );

    let task = invocation.join("task.yaml");
    fs::write(
        &task,
        format!(
            r#"
goal: "Launch from a foreign cwd"
workspace: "{}"
agents:
  echo:
    binary: /bin/echo
steps:
  - agent: echo
    task: "interactive foreign hello"
"#,
            workspace.display()
        ),
    )
    .unwrap();

    let prepare = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&invocation)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("run", &prepare);
    let id = run_id(&prepare);

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let step = Command::new(niles)
        .args(["step", &id, "--index", "1"])
        .current_dir(&invocation)
        .env("NILES_HOME", &home)
        .env("PATH", &path)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("step from invocation cwd", &step);

    let workspace_run_dir = workspace.join(".niles/runs").join(&id);
    let brief = workspace_run_dir.join("steps/001-echo.context.md");
    let launch = workspace_run_dir.join("steps/001-launch.sh");
    assert!(brief.exists());
    assert!(launch.exists());
    assert!(
        !invocation
            .join(".niles/runs")
            .join(&id)
            .join("state.json")
            .exists()
    );

    let step_stdout = String::from_utf8_lossy(&step.stdout);
    assert!(step_stdout.contains(&format!("brief: {}", brief.display())));
    assert!(step_stdout.contains(&format!(
        "status_log: {}",
        workspace_run_dir.join("status.log").display()
    )));

    let brief_body = fs::read_to_string(&brief).unwrap();
    assert!(brief_body.contains(&format!("workspace: {}", workspace.display())));
    assert!(brief_body.contains(&workspace_run_dir.join("status.log").display().to_string()));
    assert!(brief_body.contains("done: step 1 <short result>"));
    assert!(brief_body.contains("failed: step 1 <reason>"));
    assert!(brief_body.contains("blocked: step 1 <blocking issues>"));
    assert!(brief_body.contains("needs-decision: step 1 <decision needed>"));

    let launch_body = fs::read_to_string(&launch).unwrap();
    assert!(launch_body.contains(&brief.display().to_string()));

    let tmux = fs::read_to_string(&tmux_log).unwrap();
    assert!(tmux.contains("new-window -d -t niles: -n niles-echo-step-s1-"));
    assert!(tmux.contains(&format!("-c {}", workspace.display())));
    assert!(tmux.contains(" sh "));
    assert!(tmux.contains(&launch.display().to_string()));

    let status = Command::new(niles)
        .args(["status", &id])
        .current_dir(&invocation)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("status from invocation cwd", &status);
    assert!(String::from_utf8_lossy(&status.stdout).contains("1,agent,echo,running,-"));
}

#[test]
fn step_launch_failure_appends_failed_backstop() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-step-launch-failure");

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_executable(
        &bin.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  has-session) exit 1 ;;
  list-windows) exit 0 ;;
  new-window)
    printf 'create window failed: index 1 in use\n' >&2
    exit 1
    ;;
  *) exit 0 ;;
esac
"#,
    );

    let task = write_task(
        &workspace,
        r#"
goal: "Launch failure"
agents:
  echo:
    binary: /bin/echo
steps:
  - agent: echo
    task: "interactive hello"
"#,
    );

    let prepare = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
        .output()
        .unwrap();
    assert_command_success("run", &prepare);
    let id = run_id(&prepare);

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let step = Command::new(niles)
        .args(["step", "latest", "--index", "1"])
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
        .env("PATH", &path)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(!step.status.success());
    assert!(String::from_utf8_lossy(&step.stderr).contains("new-window"));

    let run_dir = workspace.join(".niles/runs").join(&id);
    let status_log = fs::read_to_string(run_dir.join("status.log")).unwrap();
    assert!(status_log.contains("failed: step 1 launch error:"));

    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
        .output()
        .unwrap();
    assert_command_success("status", &status);
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("status: failed"));
    assert!(status_stdout.contains("1,agent,echo,failed,-"));
}

#[test]
fn step_guards_block_out_of_order_and_command_steps() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-step-guard");

    let task = write_task(
        &workspace,
        r#"
goal: "Guard checks"
agents:
  echo:
    binary: /bin/echo
steps:
  - agent: echo
    task: "first"
  - command: pwd
commands:
  pwd: pwd
"#,
    );

    let prepare = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
        .output()
        .unwrap();
    assert!(prepare.status.success());

    // Ordering guard: step 2 cannot launch while step 1 is still pending.
    let out_of_order = Command::new(niles)
        .args(["step", "--index", "2"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(!out_of_order.status.success());
    assert!(
        String::from_utf8_lossy(&out_of_order.stderr).contains("prior steps must complete first")
    );

    // Complete step 1 (captured), then step 2 is a command -> directed to exec-step.
    let step1 = Command::new(niles)
        .args(["exec-step", "latest", "1"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(step1.status.success());

    let command_step = Command::new(niles)
        .args(["step", "--index", "2"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(!command_step.status.success());
    assert!(
        String::from_utf8_lossy(&command_step.stderr)
            .contains("run it captured with `niles exec-step")
    );
}

#[test]
fn step_close_marks_step_completed() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-step-close");

    let task = write_task(
        &workspace,
        r#"
goal: "Close checks"
agents:
  echo:
    binary: /bin/echo
steps:
  - agent: echo
    task: "only step"
"#,
    );

    let prepare = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
        .output()
        .unwrap();
    assert!(prepare.status.success());
    let id = run_id(&prepare);

    let waiter = Command::new(niles)
        .args([
            "wait",
            "latest",
            "--index",
            "1",
            "--interval",
            "0.05",
            "--timeout",
            "5",
        ])
        .current_dir(&workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(100));

    // step-close finalizes the step and reports completion. Window teardown is
    // best-effort (no live window in the test env) and must not fail the call.
    let close = Command::new(niles)
        .args(["step-close", "--index", "1"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(
        close.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&close.stdout),
        String::from_utf8_lossy(&close.stderr)
    );
    let close_stdout = String::from_utf8_lossy(&close.stdout);
    assert!(close_stdout.contains("step 1: completed"));
    assert!(close_stdout.contains("status: completed"));

    let started = Instant::now();
    let wait = waiter.wait_with_output().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "wait did not return promptly; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&wait.stdout),
        String::from_utf8_lossy(&wait.stderr)
    );
    assert_command_success("wait for step-close", &wait);
    assert_eq!(String::from_utf8_lossy(&wait.stdout), "closed: step 1\n");

    let status_log =
        fs::read_to_string(workspace.join(".niles/runs").join(&id).join("status.log")).unwrap();
    assert!(status_log.contains("closed: step 1"));

    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("status: completed"));
    assert!(status_stdout.contains("1,agent,echo,completed,0"));
}

#[test]
fn step_add_appends_to_run_and_reopens() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-step-add");

    let task = write_task(
        &workspace,
        r#"
goal: "step-add"
agents:
  echo:
    binary: /bin/echo
steps:
  - agent: echo
    task: "first"
    role: worker
commands:
  check:
    run: "true"
"#,
    );

    let run = String::from_utf8(
        Command::new(niles)
            .arg("run")
            .arg(&task)
            .current_dir(&workspace)
            .env("NILES_HOME", niles_home(&workspace))
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(run.contains("status: created"));

    // Complete the only step so the run reaches a terminal state.
    let step1 = Command::new(niles)
        .args(["exec-step", "latest", "1"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(step1.status.success());
    assert!(String::from_utf8_lossy(&step1.stdout).contains("status: completed"));

    // Append a review cycle: a reviewer agent step and a validation command step.
    let add_review = Command::new(niles)
        .args([
            "step-add",
            "latest",
            "--agent",
            "echo",
            "--role",
            "reviewer",
            "review it",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(
        add_review.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&add_review.stderr)
    );
    assert!(
        String::from_utf8_lossy(&add_review.stdout).contains("added: step 2 reviewer agent echo")
    );

    let add_check = Command::new(niles)
        .args([
            "step-add",
            "latest",
            "--command",
            "check",
            "--role",
            "validation",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(add_check.status.success());
    assert!(
        String::from_utf8_lossy(&add_check.stdout)
            .contains("added: step 3 validation command check")
    );

    // The run reopened to running with the two new pending steps.
    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("status: running"));
    assert!(status_stdout.contains("1,worker,agent,echo,completed,0"));
    assert!(status_stdout.contains("2,reviewer,agent,echo,pending,-"));
    assert!(status_stdout.contains("3,validation,command,check,pending,-"));

    // The appended steps are persisted to the task spec for step/exec-step.
    let task_body = fs::read_to_string(&task).unwrap();
    assert!(task_body.contains("role: reviewer"));
    assert!(task_body.contains("role: validation"));
}

#[test]
fn step_add_command_does_not_load_project_config() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-step-add-command-raw-task");

    let task = write_task(
        &workspace,
        r#"
goal: "step-add command raw task"
steps:
  - command: check
commands:
  check: "true"
"#,
    );
    let home = niles_home(&workspace);

    let run = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("run command-only task", &run);

    fs::write(workspace.join("niles.yaml"), "agents: [").unwrap();

    let add_check = Command::new(niles)
        .args(["step-add", "latest", "--command", "check"])
        .current_dir(&workspace)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("step-add command with invalid project config", &add_check);
    assert!(String::from_utf8_lossy(&add_check.stdout).contains("added: step 2 command check"));

    let task_body = fs::read_to_string(&task).unwrap();
    assert!(task_body.contains("- command: check"));
}

#[test]
fn step_add_validates_agent_with_project_config() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-step-add-project-config");

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let codex = bin.join("configured-codex");
    write_executable(
        &codex,
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.142.4\n'; exit 0 ;;
  --help) printf 'configured codex help\n'; exit 0 ;;
esac
printf 'configured codex ok\n'
"#,
    );
    fs::write(
        workspace.join("niles.yaml"),
        format!(
            r#"
agents:
  codex:
    binary: "{}"
"#,
            codex.display()
        ),
    )
    .unwrap();

    let task = write_task(
        &workspace,
        r#"
goal: "step-add project config"
steps:
  - command: check
commands:
  check: "true"
"#,
    );
    let home = niles_home(&workspace);

    let analyze = Command::new(niles)
        .args(["analyze", "--agent", "codex:omega:xhigh"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert_command_success("analyze configured codex", &analyze);

    let run = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("run command-only task", &run);

    let add_review = Command::new(niles)
        .args([
            "step-add",
            "latest",
            "--agent",
            "codex:omega:xhigh",
            "review it",
        ])
        .current_dir(&workspace)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("step-add configured codex", &add_review);
    assert!(
        String::from_utf8_lossy(&add_review.stdout)
            .contains("added: step 2 agent codex:omega:xhigh")
    );
}
