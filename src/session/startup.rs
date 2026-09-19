use anyhow::Result;
use camino::Utf8Path;

use crate::store;

/// The workers the lead is inheriting, read from the workspace it is starting in.
///
/// This used to scan for `.niles/worker/<id>.json` files, a layout that stopped existing when a
/// worker became a directory — so it reported `worker: none` in every session and the lead began
/// blind to whatever was already running. It now reads the same worker locations the rest of the
/// CLI does, so there is no second layout to drift out of step.
pub(super) fn startup_context(workspace: &Utf8Path) -> Result<String> {
    let mut ids = store::resolve_worker_locations_in(workspace)?
        .into_iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    ids.sort();

    if ids.is_empty() {
        return Ok("worker: none".to_owned());
    }
    Ok(format!("worker: {}", ids.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::temp_test_path;
    use super::*;

    use std::fs;

    /// Regression: a worker is a directory. The previous implementation looked for `<id>.json`
    /// files and its test wrote them, so the test passed while every real session saw nothing.
    #[test]
    fn startup_context_lists_worker_directories() {
        let root = temp_test_path("startup-context-workers");
        let workers = root.join(".niles/worker");
        fs::create_dir_all(workers.join("beta")).unwrap();
        fs::create_dir_all(workers.join("alpha")).unwrap();
        // The archive is a sibling directory, not a worker.
        fs::create_dir_all(workers.join("archive")).unwrap();

        assert_eq!(startup_context(&root).unwrap(), "worker: alpha, beta");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn startup_context_reports_none_for_an_empty_workspace() {
        let root = temp_test_path("startup-context-empty");
        fs::create_dir_all(root.join(".niles/worker")).unwrap();

        assert_eq!(startup_context(&root).unwrap(), "worker: none");

        fs::remove_dir_all(root).unwrap();
    }
}
