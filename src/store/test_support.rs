use std::{
    env,
    ffi::OsString,
    fs,
    sync::{Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use camino::{Utf8Path, Utf8PathBuf};

use super::{
    global::write_global_worker_pointer as store_write_global_worker_pointer,
    worker::{WorkerPointer, WorkerResolver},
};

static ENV_LOCK: Mutex<()> = Mutex::new(());

pub(super) struct ScopedEnv {
    _lock: MutexGuard<'static, ()>,
    previous_home: Option<OsString>,
    previous_niles_home: Option<OsString>,
}

impl ScopedEnv {
    pub(super) fn new(niles_home: &Utf8Path, home: &Utf8Path) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let previous_home = env::var_os("HOME");
        let previous_niles_home = env::var_os("NILES_HOME");
        set_env("NILES_HOME", niles_home.as_str());
        set_env("HOME", home.as_str());
        Self {
            _lock: lock,
            previous_home,
            previous_niles_home,
        }
    }
}

impl Drop for ScopedEnv {
    fn drop(&mut self) {
        restore_env("HOME", &self.previous_home);
        restore_env("NILES_HOME", &self.previous_niles_home);
    }
}

pub(super) struct TempDir {
    path: Utf8PathBuf,
}

impl TempDir {
    pub(super) fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = Utf8PathBuf::from_path_buf(env::temp_dir().join(format!(
            "niles-store-{label}-{}-{nanos}",
            std::process::id()
        )))
        .unwrap();
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub(super) fn path(&self) -> &Utf8Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(super) fn worker_resolver_at(local_workers_dir: &Utf8Path) -> WorkerResolver {
    let local_workspace = local_workers_dir
        .parent()
        .and_then(Utf8Path::parent)
        .unwrap()
        .to_path_buf();
    WorkerResolver {
        local_workers_dir: local_workers_dir.to_path_buf(),
        local_workspace,
    }
}

pub(super) fn assert_worker_resolves_to(
    local_workers_dir: &Utf8Path,
    worker: &str,
    worker_dir: &Utf8Path,
) {
    assert_eq!(
        worker_resolver_at(local_workers_dir)
            .named(worker)
            .unwrap()
            .unwrap()
            .worker_dir,
        worker_dir
    );
}

pub(super) fn create_dir(path: Utf8PathBuf) -> Utf8PathBuf {
    fs::create_dir_all(&path).unwrap();
    path
}

pub(super) fn write_global_worker_pointer(pointer: &WorkerPointer) -> anyhow::Result<()> {
    store_write_global_worker_pointer(pointer)
}

pub(super) fn worker_pointer(worker: &str, worker_dir: &Utf8Path) -> WorkerPointer {
    WorkerPointer {
        id: worker.to_owned(),
        workspace: worker_dir
            .parent()
            .and_then(Utf8Path::parent)
            .and_then(Utf8Path::parent)
            .unwrap_or(Utf8Path::new("/"))
            .to_path_buf(),
        worker_dir: worker_dir.to_path_buf(),
        local_stores: worker_dir
            .parent()
            .map(|parent| vec![parent.to_path_buf()])
            .unwrap_or_default(),
    }
}

fn set_env(key: &str, value: &str) {
    // SAFETY: These tests hold ENV_LOCK for all NILES_HOME/HOME mutations and resolver calls.
    // They do not spawn additional threads while the scoped environment is active.
    unsafe {
        env::set_var(key, value);
    }
}

fn restore_env(key: &str, value: &Option<OsString>) {
    // SAFETY: ScopedEnv still holds ENV_LOCK while restoring the variables it changed.
    unsafe {
        match value {
            Some(value) => env::set_var(key, value),
            None => env::remove_var(key),
        }
    }
}
