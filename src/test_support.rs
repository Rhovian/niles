//! Test-only scaffolding shared across the crate.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};

pub(crate) fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(seconds, 0).unwrap()
}

/// A unique, labeled path under the system temp directory.
pub(crate) fn temp_test_path(label: &str) -> Utf8PathBuf {
    // The clock alone collides across parallel tests: macOS reports microsecond resolution.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
        "niles-test-{label}-{}-{nanos}-{sequence}",
        std::process::id()
    )))
    .unwrap()
}

pub(crate) fn project_config(body: &str) -> anyhow::Result<crate::config::spec::ProjectConfig> {
    let root = temp_test_path("project-config");
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join("niles.yaml"), body)?;
    let result = crate::config::spec::load_project_config_from(&root);
    std::fs::remove_dir_all(root)?;
    result
}
