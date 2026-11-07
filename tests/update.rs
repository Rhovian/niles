#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::{fs, os::unix::fs::symlink, path::Path, process::Command};

fn installed_update(home: &Path, layout: &str) -> Command {
    let bin = home.join(layout);
    fs::create_dir_all(&bin).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_niles"), bin.join("niles")).unwrap();
    let mut command = Command::new(bin.join("niles"));
    command.arg("update").env("HOME", home);
    command
}

#[test]
fn update_executes_sibling_and_preserves_status() {
    let root = tempfile::tempdir().unwrap();
    let mut command = installed_update(root.path(), "bin");
    common::write_executable(&root.path().join("niles-update"), "#!/bin/sh\nexit 99\n");
    common::write_executable(
        &root.path().join("bin/niles-update"),
        "#!/bin/sh\nprintf 'updater ran\\n'\nprintf 'diagnostic\\n' >&2\nexit 7\n",
    );
    let output = command
        .env("PATH", root.path())
        .current_dir(root.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stdout, b"updater ran\n");
    assert_eq!(output.stderr, b"diagnostic\n");
}

#[test]
fn update_reports_package_manager_or_missing_updater() {
    let root = tempfile::tempdir().unwrap();
    let brew = installed_update(root.path(), "Cellar/niles/1.0/bin");
    let link = root.path().join("niles-link");
    symlink(brew.get_program(), &link).unwrap();
    let output = Command::new(link).arg("update").output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"Run: brew upgrade niles\n");
    assert!(output.stderr.is_empty());

    let mut cargo = installed_update(root.path(), ".cargo/bin");
    let metadata = root.path().join(".cargo/.crates.toml");
    fs::write(&metadata, "[v1]\n'niles 1.0.0 (x)' = ['niles']\n").unwrap();
    let output = cargo.output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"Run: cargo install niles --locked\n");
    assert!(output.stderr.is_empty());

    fs::write(&metadata, "[v1]\n'other 1.0.0 (x)' = ['other']\n").unwrap();
    let output = cargo.output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("rerun the shell installer"));
    fs::remove_file(&metadata).unwrap();
    let missing = cargo.output().unwrap();
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(missing.stdout, output.stdout);
    assert_eq!(missing.stderr, output.stderr);

    let mut custom = installed_update(root.path(), "cargo root's/bin");
    fs::write(
        root.path().join("cargo root's/.crates.toml"),
        "[v1]\n'niles 1.0.0 (x)' = ['niles']\n",
    )
    .unwrap();
    let output = custom.output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "Run: cargo install niles --locked --root '{}/cargo root'\\''s'\n",
            root.path().canonicalize().unwrap().display()
        )
    );
    assert!(output.stderr.is_empty());
}
