#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::{assert_command_success, stdout_of};

use std::{fs, os::unix::fs::symlink, path::Path, process::Command};

fn install_root() -> tempfile::TempDir {
    // Hard links require the fixture and built executable to share a filesystem.
    tempfile::tempdir_in(Path::new(env!("CARGO_BIN_EXE_niles")).parent().unwrap()).unwrap()
}

fn installed_update(home: &Path, layout: &str) -> Command {
    let bin = home.join(layout);
    fs::create_dir_all(&bin).unwrap();
    // A concurrent fork can inherit a copy's write fd and cause ETXTBSY on exec.
    fs::hard_link(env!("CARGO_BIN_EXE_niles"), bin.join("niles")).unwrap();
    let mut command = Command::new(bin.join("niles"));
    command.arg("update").env("HOME", home);
    command
}

fn assert_prints(output: &std::process::Output, expected: &str) {
    assert_command_success("update", output);
    assert_eq!(stdout_of(output), expected);
    assert!(output.stderr.is_empty());
}

#[test]
fn update_executes_sibling_and_preserves_status() {
    let root = install_root();
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
    let root = install_root();
    let brew = installed_update(root.path(), "Cellar/niles/1.0/bin");
    let link = root.path().join("niles-link");
    symlink(brew.get_program(), &link).unwrap();
    let output = Command::new(link).arg("update").output().unwrap();
    assert_prints(&output, "Run: brew upgrade niles\n");

    let mut cargo = installed_update(root.path(), ".cargo/bin");
    let metadata = root.path().join(".cargo/.crates.toml");
    fs::write(&metadata, "[v1]\n'niles 1.0.0 (x)' = ['niles']\n").unwrap();
    let output = cargo.output().unwrap();
    assert_prints(&output, "Run: cargo install niles --locked\n");

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
    assert_prints(
        &output,
        &format!(
            "Run: cargo install niles --locked --root '{}/cargo root'\\''s'\n",
            root.path().canonicalize().unwrap().display()
        ),
    );
}

#[test]
fn update_channel_selects_updater_arguments() {
    for (args, expected) in [
        (vec![], "\n"),
        (vec!["--channel", "nightly"], "--prerelease\n"),
    ] {
        let root = install_root();
        let mut command = installed_update(root.path(), "bin");
        common::write_executable(
            &root.path().join("bin/niles-update"),
            "#!/bin/sh\nprintf '%s\\n' \"$@\"\n",
        );
        let output = command.args(&args).output().unwrap();
        assert_prints(&output, expected);
    }
}

#[test]
fn nightly_requires_shell_updater() {
    let root = install_root();
    for layout in ["Cellar/niles/1.0/bin", ".cargo/bin", "bin"] {
        let mut command = installed_update(root.path(), layout);
        if layout.starts_with("Cellar/") {
            common::write_executable(
                &root.path().join(layout).join("niles-update"),
                "#!/bin/sh\nexit 0\n",
            );
        }
        let output = command.args(["--channel", "nightly"]).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("nightly updates require"));
    }
}
