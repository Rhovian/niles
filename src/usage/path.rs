use camino::{Utf8Path, Utf8PathBuf};

const USAGE_FILE: &str = "usage.json";

pub(crate) fn worker_usage_path(worker_dir: &Utf8Path) -> Utf8PathBuf {
    worker_dir.join(USAGE_FILE)
}
