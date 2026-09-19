use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Output,
    time::{SystemTime, UNIX_EPOCH},
};

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
pub fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[allow(dead_code)]
pub fn write_workspace_manifest(workspace: &Path, lead: &str, worker: &str, reviewer: &str) {
    fs::create_dir_all(workspace.join(".niles")).unwrap();
    fs::write(
        workspace.join(".niles/manifest.yaml"),
        format!("lead: {lead}\nworker: {worker}\nreviewer: {reviewer}\nniles_schema: 2\n"),
    )
    .unwrap();
}
