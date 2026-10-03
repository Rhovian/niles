#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{TestEnv, assert_command_success, stdout_of};
use std::{fs, os::unix::fs::symlink};

#[test]
fn prune_previews_then_removes_only_eligible_metadata_across_projects() {
    let env = TestEnv::new("niles-prune");
    let registry = env.home.join(".niles/projects");
    let missing = env.root.join("missing");
    let other = env.root.join("other");
    fs::create_dir_all(&registry).unwrap();
    fs::create_dir(&other).unwrap();
    for (name, target) in [
        ("a-missing", &missing),
        ("b-live", &env.root),
        ("c-other", &other),
    ] {
        symlink(target, registry.join(name)).unwrap();
    }
    let archive = env.root.join(".niles/worker/archive");
    let sessions = other.join(".niles/sessions");
    let removed = [
        registry.join("a-missing"),
        archive.join("auth-fix-20000101T000000000000000Z"),
        sessions.join("20000101T000000000000000Z"),
    ];
    let kept = [
        archive.join(format!(
            "new-{}",
            chrono::Utc::now().format("%Y%m%dT%H%M%S%fZ")
        )),
        archive.join("unparseable"),
        sessions.join("20000102T000000000000000Z"),
        env.root.join(".niles/worker/live"),
    ];
    for dir in removed[1..].iter().chain(&kept) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("payload"), "keep or remove with directory").unwrap();
    }
    let files = [
        archive.join("file-20000101T000000000000000Z"),
        sessions.join("20000103T000000000000000Z"),
        sessions.join("latest"),
        env.root.join(".niles/manifest.yaml"),
    ];
    for file in &files {
        fs::write(file, "keep").unwrap();
    }
    let links = [
        archive.join("link-20000101T000000000000000Z"),
        sessions.join("20000104T000000000000000Z"),
    ];
    for link in &links {
        symlink(&kept[1], link).unwrap();
    }
    for apply in [false, true] {
        let mut command = env.niles(&env.root, &["prune"]);
        command.env("HOME", &env.home);
        if apply {
            command.arg("--apply");
        } else {
            command.args(["--older-than", "14"]);
        }
        let output = command.output().unwrap();
        assert_command_success("prune", &output);
        let action = if apply { "removed" } else { "would remove" };
        let expected: String = removed
            .iter()
            .map(|path| format!("{action} {}\n", path.display()))
            .collect();
        assert_eq!(stdout_of(&output), expected);
        for path in &removed {
            assert_eq!(fs::symlink_metadata(path).is_ok(), !apply);
        }
        for path in kept.iter().chain(&files).chain(&links) {
            assert!(path.exists(), "{}", path.display());
        }
        assert!(!missing.exists());
        assert_eq!(fs::read_link(registry.join("b-live")).unwrap(), env.root);
        assert_eq!(fs::read_link(registry.join("c-other")).unwrap(), other);
    }
    let output = env
        .niles(&env.root, &["prune"])
        .env("HOME", &env.home)
        .output()
        .unwrap();
    assert_command_success("empty prune", &output);
    assert!(output.stdout.is_empty());
}
