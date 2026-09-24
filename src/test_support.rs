//! Test-only scaffolding the whole crate shares.
//!
//! A module's own `test_support` keeps what only its tests use — this is the one piece every
//! module's tests want and none of them should write a second time. A path under the system temp
//! directory is the whole of it.

use std::time::{SystemTime, UNIX_EPOCH};

use camino::Utf8PathBuf;

/// A unique path under the system temp directory, named for the test that asked for it.
///
/// A path, not a directory: the caller that wants `create_dir_all` says so, and one that is only
/// naming a file it will write is not made to clean up a directory it never needed. The label keeps
/// a leftover readable at a glance; the timestamp and the pid keep two runs of the same test apart.
pub(crate) fn temp_test_path(label: &str) -> Utf8PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    Utf8PathBuf::from_path_buf(
        std::env::temp_dir().join(format!("niles-test-{label}-{}-{nanos}", std::process::id())),
    )
    .unwrap()
}
