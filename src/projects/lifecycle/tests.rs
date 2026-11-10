use super::*;
use crate::store;
use crate::test_support::{remove_workspace, temp_dir, write_worker_meta};
use std::fs;

#[test]
fn concurrent_close_returns_not_found_only_after_metadata_disappears() {
    let root = temp_dir("concurrent-close");
    let name = registry::ProjectName::parse(root.file_name().unwrap()).unwrap();
    registry::register(&name, &root).unwrap();
    let dir = store::workers_dir(&root).unwrap().join("w");
    let created_at = "2026-01-01T00:00:00Z".parse().unwrap();
    write_worker_meta(&root, "w", "api:w", created_at, None);
    assert!(close_worker(name.as_str(), "w", &|_, _| bail!("still live")).is_err());
    let closing_dir = dir.clone();
    assert!(matches!(
        close_worker(name.as_str(), "w", &move |_, args| {
            assert_eq!(args, ["close", "w"]);
            fs::remove_file(closing_dir.join("meta.json"))?;
            bail!("concurrent close won")
        })
        .unwrap(),
        Closed::NotFound
    ));
    assert!(matches!(
        close_worker(name.as_str(), "--all", &|_, _| anyhow::bail!(
            "must not execute"
        ))
        .unwrap(),
        Closed::NotFound
    ));
    fs::remove_file(registry::directory().unwrap().join(name.as_str())).unwrap();
    remove_workspace(&root).unwrap();
}
