//! Test-only scaffolding shared across the crate.

use std::time::{SystemTime, UNIX_EPOCH};

use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};

pub(crate) fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(seconds, 0).unwrap()
}

/// A unique, labeled path under the system temp directory.
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
