use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn run_executes_steps_and_persists_state() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
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
    )
    .unwrap();

    let output = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("status: completed"));
    assert!(stdout.contains("hello test"));

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
    assert_eq!(String::from_utf8_lossy(&log.stdout), "hello test\n");

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
fn run_captures_git_diff_after_each_step() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-diff-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

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

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Capture diff"
steps:
  - command: edit
commands:
  edit: printf 'after\n' > tracked.txt
"#,
    )
    .unwrap();

    let output = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

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
    let workspace = std::env::temp_dir().join(format!(
        "niles-config-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

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

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Use project config"
steps:
  - agent: echo
    task: "configured agent"
  - command: marker
"#,
    )
    .unwrap();

    let output = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let first_log = Command::new(niles)
        .arg("log")
        .arg("--step")
        .arg("1")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(first_log.status.success());
    assert_eq!(
        String::from_utf8_lossy(&first_log.stdout),
        "configured agent\n"
    );

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
fn run_prints_actionable_failure_summary() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-failure-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Fail usefully"
steps:
  - command: fail
commands:
  fail:
    run: "for i in 1 2 3 4 5 6 7 8 9 10 11 12 13; do echo tail-line-$i >&2; done; exit 7"
"#,
    )
    .unwrap();

    let output = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("status: failed"));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failure:"));
    assert!(stderr.contains("step: 1 command fail"));
    assert!(stderr.contains("exit: 7"));
    assert!(stderr.contains("stderr: .niles/runs/"));
    assert!(stderr.contains("diff: .niles/runs/"));
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
fn manifest_generates_runnable_role_workflow() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-manifest-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    fs::write(
        workspace.join("niles.yaml"),
        r#"
agents:
  echo:
    binary: /bin/echo
commands:
  test:
    run: printf 'manifest command\n'
"#,
    )
    .unwrap();

    let output = Command::new(niles)
        .args([
            "manifest",
            "--project",
            ".",
            "--planner",
            "echo",
            "--implementer",
            "echo",
            "--reviewer",
            "echo",
            "--command",
            "test",
            "Ship",
            "role",
            "workflow",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let manifest_line = stdout
        .lines()
        .find(|line| line.starts_with("manifest: "))
        .expect("manifest output should include manifest path");
    let manifest_path = workspace.join(manifest_line.trim_start_matches("manifest: "));
    let manifest = fs::read_to_string(&manifest_path).unwrap();

    assert!(manifest.contains("goal: Ship role workflow"));
    assert!(manifest.contains("workspace: ."));
    assert!(manifest.contains("role: planner"));
    assert!(manifest.contains("role: implementer"));
    assert!(manifest.contains("role: reviewer"));
    assert!(manifest.contains("role: validation"));
    assert!(manifest.contains("agent: echo"));
    assert!(manifest.contains("command: test"));
    assert!(manifest.contains("manifest command"));

    let run = Command::new(niles)
        .arg("run")
        .arg(&manifest_path)
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );

    let run_stdout = String::from_utf8_lossy(&run.stdout);
    assert!(run_stdout.contains("step 1: planner agent echo"));
    assert!(run_stdout.contains("step 3: validation command test"));
    assert!(run_stdout.contains("manifest command"));
    assert!(run_stdout.contains("status: completed"));

    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(status.status.success());

    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("steps[6]{index,role,kind,label,status,exit}:"));
    assert!(status_stdout.contains("1,planner,agent,echo,completed,0"));
    assert!(status_stdout.contains("2,implementer,agent,echo,completed,0"));
    assert!(status_stdout.contains("3,validation,command,test,completed,0"));
    assert!(status_stdout.contains("4,reviewer,agent,echo,completed,0"));
}
