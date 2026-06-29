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

    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(status.status.success());

    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("\"status\": \"completed\""));
    assert!(status_stdout.contains("001-echo.stdout.txt"));

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
