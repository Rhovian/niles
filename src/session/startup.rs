use anyhow::Result;
use camino::Utf8Path;

use crate::store;

/// The workers the lead is inheriting, read through the same store interface as the rest of the CLI.
pub(super) fn startup_context(workspace: &Utf8Path) -> Result<String> {
    let ids = store::worker_locations(workspace)?
        .into_iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Ok("worker: none".to_owned());
    }
    Ok(format!("worker: {}", ids.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_path;

    use std::fs;

    /// Worker directories are the source of startup context.
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
