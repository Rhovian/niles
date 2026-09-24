use std::{fs, os::unix::fs::PermissionsExt};

use camino::Utf8Path;

pub(super) use crate::test_support::temp_test_path;

pub(super) fn write_executable_script(path: &Utf8Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

pub(super) fn shell_quote(path: &Utf8Path) -> String {
    format!("'{}'", path.as_str().replace('\'', "'\\''"))
}
