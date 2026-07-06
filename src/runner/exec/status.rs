use anyhow::Result;
use camino::Utf8Path;

use crate::{util::append_line, wake};

pub(in crate::runner::exec) fn append_run_status(run_dir: &Utf8Path, line: &str) -> Result<()> {
    let path = wake::status_log_path(run_dir);
    append_line(
        &path,
        line,
        |path| format!("failed to open {path}"),
        |path| format!("failed to inspect {path} before status append"),
        |path| format!("failed to write {path}"),
    )
}
