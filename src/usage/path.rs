use camino::{Utf8Path, Utf8PathBuf};

use crate::util::slugify;

const USAGE_FILE: &str = "usage.json";

pub(crate) fn worker_usage_path(worker_dir: &Utf8Path) -> Utf8PathBuf {
    worker_dir.join(USAGE_FILE)
}

pub(crate) fn step_usage_path(steps_dir: &Utf8Path, index: usize, label: &str) -> Utf8PathBuf {
    steps_dir.join(format!("{index:03}-{}.usage.json", slugify(label)))
}
