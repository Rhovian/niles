use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

#[allow(dead_code)]
pub fn prepare_run(niles: &str, workspace: &Path, task: &Path) -> Output {
    let output = Command::new(niles)
        .arg("run")
        .arg(task)
        .current_dir(workspace)
        .env("NILES_HOME", niles_home(workspace))
        .output()
        .unwrap();
    assert_command_success("run", &output);
    output
}

#[allow(dead_code)]
pub fn exec_step_output(niles: &str, workspace: &Path, index: usize) -> Output {
    Command::new(niles)
        .arg("exec-step")
        .arg("latest")
        .arg(index.to_string())
        .current_dir(workspace)
        .output()
        .unwrap()
}

#[allow(dead_code)]
pub fn drive_exec_steps(
    niles: &str,
    workspace: &Path,
    steps: impl IntoIterator<Item = usize>,
) -> Vec<Output> {
    let mut outputs = Vec::new();
    for index in steps {
        let output = exec_step_output(niles, workspace, index);
        assert_command_success(&format!("exec-step {index}"), &output);
        outputs.push(output);
    }
    outputs
}

pub fn assert_command_success(label: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{label} stdout:\n{}\n{label} stderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[allow(dead_code)]
pub fn niles_home(workspace: &Path) -> PathBuf {
    workspace.join(".niles-test-home")
}

pub fn temp_workspace(prefix: &str) -> PathBuf {
    let workspace = std::env::temp_dir().join(format!(
        "{prefix}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();
    workspace
}

#[allow(dead_code)]
pub fn write_task(workspace: &Path, body: &str) -> PathBuf {
    let task = workspace.join("task.yaml");
    fs::write(&task, body).unwrap();
    task
}

#[allow(dead_code)]
pub fn run_id(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("run: "))
        .expect("run output should include run id")
        .to_owned()
}

#[allow(dead_code)]
pub fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[allow(dead_code)]
pub fn write_workspace_manifest(
    workspace: &Path,
    manager: &str,
    planner: &str,
    implementer: &str,
    reviewer: &str,
    validation_command: &str,
) {
    fs::create_dir_all(workspace.join(".niles")).unwrap();
    fs::write(
        workspace.join(".niles/manifest.yaml"),
        format!(
            "manager: {manager}\nplanner: {planner}\nimplementer: {implementer}\nreviewer: {reviewer}\nvalidation_command: {validation_command}\n"
        ),
    )
    .unwrap();
}
