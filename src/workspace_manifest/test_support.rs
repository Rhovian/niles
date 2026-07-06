use std::time::{SystemTime, UNIX_EPOCH};

use camino::Utf8PathBuf;

pub(in crate::workspace_manifest) fn temp_test_path(label: &str) -> Utf8PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
        "niles-workspace-manifest-{label}-{}-{nanos}",
        std::process::id()
    )))
    .unwrap()
}
