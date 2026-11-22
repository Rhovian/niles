#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
use common::{assert_command_success, atomic_write, stdout_of};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::{Command, Output};

mod common;

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

fn assert_prints(output: &Output, expected: &str) {
    assert_command_success("update", output);
    assert_eq!(stdout_of(output), expected);
    assert!(output.stderr.is_empty());
}

#[test]
fn update_preserves_remote_or_executes_sibling() {
    for (args, expected) in [
        (&[][..], "\n"),
        (&["--channel", "nightly"][..], "--prerelease\n"),
    ]
    .into_iter()
    .take(if cfg!(feature = "remote") { 1 } else { 2 })
    {
        let root = install_root();
        let mut command = installed_update(root.path(), "bin");
        common::write_executable(&root.path().join("niles-update"), "#!/bin/sh\nexit 99\n");
        common::write_executable(
            &root.path().join("bin/niles-update"),
            "#!/bin/sh\nprintf '%s\\n' \"$@\"\nprintf 'diagnostic\\n' >&2\nexit 7\n",
        );
        let output = command
            .args(args)
            .env("PATH", root.path())
            .output()
            .unwrap();
        if cfg!(feature = "remote") {
            assert_eq!(output.status.code(), Some(1));
            assert!(output.stdout.is_empty());
            assert!(String::from_utf8_lossy(&output.stderr).contains("--features remote"));
        } else {
            assert_eq!(output.status.code(), Some(7));
            assert_eq!(output.stdout, expected.as_bytes());
            assert_eq!(output.stderr, b"diagnostic\n");
        }
    }
}

#[test]
fn update_reports_package_manager_or_missing_updater() {
    let root = install_root();
    if !cfg!(feature = "remote") {
        let brew = installed_update(root.path(), "Cellar/niles/1.0/bin");
        let link = root.path().join("niles-link");
        symlink(brew.get_program(), &link).unwrap();
        let output = Command::new(link).arg("update").output().unwrap();
        assert_prints(&output, "Run: brew upgrade niles\n");
    }
    let cargo_command = if cfg!(feature = "remote") {
        "cargo install niles --locked --features remote"
    } else {
        "cargo install niles --locked"
    };

    let mut cargo = installed_update(root.path(), ".cargo/bin");
    let metadata = root.path().join(".cargo/.crates.toml");
    atomic_write(&metadata, "[v1]\n'niles 1.0.0 (x)' = ['niles']\n").unwrap();
    let output = cargo.output().unwrap();
    assert_prints(&output, &format!("Run: {cargo_command}\n"));

    atomic_write(&metadata, "[v1]\n'other 1.0.0 (x)' = ['other']\n").unwrap();
    let output = cargo.output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let recovery = if cfg!(feature = "remote") {
        "scripts/install-remote.sh"
    } else {
        "rerun the shell installer"
    };
    assert!(String::from_utf8_lossy(&output.stderr).contains(recovery));
    fs::remove_file(&metadata).unwrap();
    let missing = cargo.output().unwrap();
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(missing.stdout, output.stdout);
    assert_eq!(missing.stderr, output.stderr);

    let mut custom = installed_update(root.path(), "cargo root's/bin");
    atomic_write(
        root.path().join("cargo root's/.crates.toml"),
        "[v1]\n'niles 1.0.0 (x)' = ['niles']\n",
    )
    .unwrap();
    let output = custom.output().unwrap();
    assert_prints(
        &output,
        &format!(
            "Run: {cargo_command} --root '{}/cargo root'\\''s'\n",
            root.path().canonicalize().unwrap().display()
        ),
    );
}

#[test]
fn nightly_requires_shell_updater() {
    let root = install_root();
    for layout in [".cargo/bin", "bin"]
        .into_iter()
        .chain((!cfg!(feature = "remote")).then_some("Cellar/niles/1.0/bin"))
    {
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
