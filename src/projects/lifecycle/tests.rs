use super::*;
use crate::{
    store,
    test_support::{remove_workspace, temp_test_path},
};
use std::fs;

#[test]
fn concurrent_close_returns_not_found_only_after_metadata_disappears() {
    let root = temp_test_path("concurrent-close");
    fs::create_dir_all(&root).unwrap();
    let name = registry::ProjectName::parse(root.file_name().unwrap()).unwrap();
    registry::register(&name, &root).unwrap();
    let dir = store::workers_dir(&root).unwrap().join("w");
    fs::create_dir_all(&dir).unwrap();
    store::write_json(&dir.join("meta.json"), &serde_json::json!({
        "id":"w", "role":"worker", "agent":"codex", "created_at":"2026-01-01T00:00:00Z",
        "project":root, "window":"api:w", "brief":dir.join("brief.md"), "launch":dir.join("launch.sh")
    })).unwrap();
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
        close_worker(name.as_str(), "--all", &|_, _| panic!("must not execute")).unwrap(),
        Closed::NotFound
    ));
    fs::remove_file(registry::directory().unwrap().join(name.as_str())).unwrap();
    remove_workspace(&root).unwrap();
}
