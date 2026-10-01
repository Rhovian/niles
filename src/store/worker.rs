use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};

use crate::util::read_dir_utf8_paths;

use super::paths::{ARCHIVE_DIR, workers_dir};

pub(crate) fn worker_location(workspace: &Utf8Path, worker: &str) -> Option<Utf8PathBuf> {
    let worker_dir = workers_dir(workspace).join(worker);
    worker_dir.is_dir().then_some(worker_dir)
}

pub(crate) fn worker_locations(workspace: &Utf8Path) -> Result<Vec<WorkerListEntry>> {
    Ok(read_dir_utf8_paths(&workers_dir(workspace))?
        .into_iter()
        .filter(|path| path.is_dir())
        .filter_map(|worker_dir| {
            let id = worker_dir.file_name()?.to_owned();
            (id != ARCHIVE_DIR).then_some(WorkerListEntry { id, worker_dir })
        })
        .collect())
}

#[derive(Clone, Debug)]
pub(crate) struct WorkerListEntry {
    pub(crate) id: String,
    pub(crate) worker_dir: Utf8PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use crate::test_support::temp_test_path;

    #[test]
    fn named_worker_resolution_uses_local_directory_only() {
        let root = temp_test_path("worker-resolution-dir-only");
        let workspace = root.join("workspace");
        let local_workers_dir = workers_dir(&workspace);
        fs::create_dir_all(&local_workers_dir).unwrap();

        let worker = "auth-fix";
        fs::write(local_workers_dir.join("auth-fix.json"), "{}").unwrap();
        assert!(worker_location(&workspace, worker).is_none());

        let worker_dir = local_workers_dir.join(worker);
        fs::create_dir_all(&worker_dir).unwrap();
        assert_eq!(
            worker_location(&workspace, worker),
            Some(worker_dir.clone())
        );

        fs::remove_dir_all(&worker_dir).unwrap();
        assert!(worker_location(&workspace, worker).is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_listing_uses_current_workspace_directories_only() {
        let root = temp_test_path("worker-local-listing");
        let workspace = root.join("workspace");
        let local_workers_dir = workers_dir(&workspace);
        fs::create_dir_all(&local_workers_dir).unwrap();
        fs::write(local_workers_dir.join("leftover.json"), "{}").unwrap();
        fs::create_dir_all(local_workers_dir.join("dir-worker")).unwrap();
        fs::create_dir_all(local_workers_dir.join(ARCHIVE_DIR)).unwrap();

        let entries = worker_locations(&workspace).unwrap();
        let ids = entries
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["dir-worker"]);
        fs::remove_dir_all(root).unwrap();
    }
}
