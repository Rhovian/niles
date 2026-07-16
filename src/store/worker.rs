use anyhow::Result;
use camino::Utf8PathBuf;

use crate::util::{current_dir_utf8, read_dir_utf8_paths};

use super::paths::{NILES_DIR, WORKERS_DIR};

pub(crate) fn resolve_worker_location(worker: &str) -> Result<Option<Utf8PathBuf>> {
    WorkerResolver::from_current()?.named(worker)
}

pub(crate) fn resolve_worker_locations() -> Result<Vec<WorkerListEntry>> {
    WorkerResolver::from_current()?.all()
}

pub(super) struct WorkerResolver {
    pub(super) local_workers_dir: Utf8PathBuf,
}

impl WorkerResolver {
    fn from_current() -> Result<Self> {
        let local_workspace = current_dir_utf8()?;
        Ok(Self {
            local_workers_dir: local_workspace.join(NILES_DIR).join(WORKERS_DIR),
        })
    }

    pub(super) fn named(&self, worker: &str) -> Result<Option<Utf8PathBuf>> {
        let worker_dir = self.local_workers_dir.join(worker);
        Ok(worker_dir.is_dir().then_some(worker_dir))
    }

    fn all(&self) -> Result<Vec<WorkerListEntry>> {
        let workers = read_dir_utf8_paths(&self.local_workers_dir)?
            .into_iter()
            .filter(|path| path.is_dir())
            .filter_map(|worker_dir| {
                let id = worker_dir.file_name()?.to_owned();
                (id != "archive").then_some(WorkerListEntry { id, worker_dir })
            })
            .collect();
        Ok(workers)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WorkerListEntry {
    pub(crate) id: String,
    pub(crate) worker_dir: Utf8PathBuf,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::store::test_support::{
        ScopedEnv, TempDir, assert_worker_resolves_to, create_dir, worker_resolver_at,
    };

    #[test]
    fn named_worker_resolution_uses_local_directory_only() {
        let root = TempDir::new("worker-resolution-dir-only");
        let _env = ScopedEnv::new(&root.path().join("niles-home"), &root.path().join("home"));
        let local_workers_dir = root.path().join("workspace/.niles/worker");
        fs::create_dir_all(&local_workers_dir).unwrap();

        let worker = "auth-fix";
        fs::write(local_workers_dir.join("auth-fix.json"), "{}").unwrap();
        assert!(
            worker_resolver_at(&local_workers_dir)
                .named(worker)
                .unwrap()
                .is_none()
        );

        let worker_dir = create_dir(local_workers_dir.join(worker));
        assert_worker_resolves_to(&local_workers_dir, worker, &worker_dir);

        fs::remove_dir_all(&worker_dir).unwrap();
        assert!(
            worker_resolver_at(&local_workers_dir)
                .named(worker)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn local_listing_uses_current_workspace_directories_only() {
        let root = TempDir::new("worker-local-listing");
        let _env = ScopedEnv::new(&root.path().join("niles-home"), &root.path().join("home"));
        let local_workers_dir = root.path().join("workspace/.niles/worker");
        fs::create_dir_all(&local_workers_dir).unwrap();
        fs::write(local_workers_dir.join("leftover.json"), "{}").unwrap();
        create_dir(local_workers_dir.join("dir-worker"));
        create_dir(local_workers_dir.join("archive"));

        let entries = worker_resolver_at(&local_workers_dir).all().unwrap();
        let ids = entries
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["dir-worker"]);
    }
}
