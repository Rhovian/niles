use std::{
    fs,
    os::unix::fs::PermissionsExt,
    time::{SystemTime, UNIX_EPOCH},
};

use camino::{Utf8Path, Utf8PathBuf};

pub(super) fn temp_test_path(label: &str) -> Utf8PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
        "niles-session-{label}-{}-{nanos}",
        std::process::id()
    )))
    .unwrap()
}

pub(super) fn write_executable_script(path: &Utf8Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

pub(super) fn shell_quote(path: &Utf8Path) -> String {
    format!("'{}'", path.as_str().replace('\'', "'\\''"))
}
