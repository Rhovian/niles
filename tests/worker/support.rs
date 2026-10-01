pub(crate) use crate::common::*;
pub(crate) use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn write_corrupt_worker_fixture(workspace: &Path, id: &str) -> PathBuf {
    let worker_dir = workspace.join(".niles/worker").join(id);
    fs::create_dir_all(&worker_dir).unwrap();
    fs::write(worker_dir.join("status.log"), "working: bad metadata\n").unwrap();
    fs::write(
        worker_dir.join("meta.json"),
        format!(
            r#"{{
  "id": "{id}",
  "agent": "codex"
}}
"#
        ),
    )
    .unwrap();
    worker_dir
}

pub(crate) fn latest_archive_dir(workspace: &Path, id: &str) -> PathBuf {
    let archive_root = workspace.join(".niles/worker/archive");
    let prefix = format!("{id}-");
    let mut archives = fs::read_dir(&archive_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix))
        })
        .collect::<Vec<_>>();
    archives.sort();
    archives.pop().expect("expected worker archive")
}

pub(crate) fn assert_archived_with_closed_sentinel(workspace: &Path, id: &str) {
    assert!(!workspace.join(".niles/worker").join(id).exists());
    let archive_dir = latest_archive_dir(workspace, id);
    assert!(archive_dir.join("meta.json").is_file());
    assert!(
        fs::read_to_string(archive_dir.join("status.log"))
            .unwrap()
            .contains(&format!("closed: {id}"))
    );
}
