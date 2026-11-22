#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]

mod common;

use common::*;
use std::fs;
#[cfg(target_os = "macos")]
use std::os::unix::fs::MetadataExt;

#[test]
fn doctor_reports_binary_identity() {
    let workspace = temp_workspace("niles-doctor-test");
    let home = niles_home(&workspace);

    let output = niles_bare(&workspace, &home)
        .arg("doctor")
        .output()
        .unwrap();

    assert_command_success("doctor", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains(concat!("binary: niles ", env!("CARGO_PKG_VERSION"), " (")));
    assert!(stdout.contains("git_hash: "));
    assert!(stdout.contains("built_at: "));
    assert!(stdout.contains("dev_mode: no"));
    assert!(stdout.contains("agents: no workspace manifest"));
    let output = run_bare(&workspace, &["--version"]);
    assert_success_contains(
        "--version",
        &output,
        concat!("niles ", env!("CARGO_PKG_VERSION"), " ("),
    );
    assert!(stdout_of(&output).contains("built "));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn doctor_compares_running_remote_service_to_installed_binary() {
    let env = TestEnv::new("niles-doctor-remote-service");
    let disabled = env.root.join("service niles");
    write_executable(
        &disabled,
        "#!/bin/sh\n[ \"$#\" -eq 1 ] && [ \"$1\" = --version ] || exit 42\nprintf 'niles 1.15.0 (fixture, built 0, features: none)\\n'\n",
    );
    let legacy = env.root.join("old niles");
    write_executable(
        &legacy,
        "#!/bin/sh\nprintf 'niles 1.15.0 (fixture, built 0)\\n'\n",
    );
    let niles = env!("CARGO_BIN_EXE_niles");
    #[cfg(target_os = "linux")]
    write_executable(
        &env.bin.join("systemctl"),
        r#"#!/bin/sh
[ "$*" = '--user show --property=LoadState,MainPID,ExecStart niles-remote.service' ] || exit 42
case "$SERVICE_PID" in
  absent) printf 'LoadState=not-found\n'; exit 0 ;;
  error) printf 'manager offline\n' >&2; exit 1 ;;
  self) pid=$PPID ;;
  *) pid=$SERVICE_PID ;;
esac
printf 'MainPID=%s\nExecStart={ path=%s ; argv[]=niles remote ; }\nLoadState=loaded\n' "$pid" "$SERVICE_PROGRAM"
"#,
    );
    #[cfg(target_os = "macos")]
    let plist = {
        write_executable(
            &env.bin.join("launchctl"),
            r#"#!/bin/sh
case "$SERVICE_PID" in
  0) exit 113 ;;
  error|absent) printf 'manager offline\n' >&2; exit 1 ;;
  self) pid=1 ;;
  *) pid=$SERVICE_PID ;;
esac
printf 'service = {\n\tprogram = %s\n\tpid = %s\n}\n' "$SERVICE_PROGRAM" "$pid"
"#,
        );
        let binary = fs::metadata(niles).unwrap();
        write_executable(
            &env.bin.join("lsof"),
            &format!(
                "#!/bin/sh\nprintf 'p%s\\000\\nftxt\\000D0x{:x}\\000i{}\\000\\n' \"$3\"\n",
                binary.dev(),
                binary.ino(),
            ),
        );
        env.home.join("Library/LaunchAgents/dev.niles.remote.plist")
    };

    let doctor = |pid: &str, program: &str| {
        let output = env
            .niles(&env.root, &["doctor"])
            .env("SERVICE_PID", pid)
            .env("SERVICE_PROGRAM", program)
            .output()
            .unwrap();
        assert_command_success("doctor", &output);
        stdout_of(&output)
    };
    let disabled = disabled.to_str().unwrap();
    let absent = doctor("absent", disabled);
    assert!(
        absent.contains("remote service: not installed\n"),
        "{absent}"
    );

    #[cfg(target_os = "macos")]
    {
        fs::create_dir_all(plist.parent().unwrap()).unwrap();
        atomic_write(&plist, format!(
            "<plist version=\"1.0\"><dict><key>ProgramArguments</key><array><string>{disabled}</string><string>remote</string></array></dict></plist>"
        )).unwrap();
    }

    let failed = doctor("error", disabled);
    assert!(failed.contains("remote service: unavailable: "), "{failed}");
    assert!(failed.contains("manager offline"), "{failed}");
    assert!(failed.contains("agents: no workspace manifest"), "{failed}");

    let built_feature = if cfg!(feature = "remote") {
        "enabled"
    } else {
        "disabled"
    };
    for (pid, program, feature, state) in [
        ("0", disabled, "disabled", "stopped"),
        ("self", niles, built_feature, "running binary matches"),
        ("self", disabled, "disabled", "running binary differs"),
        (
            "self",
            legacy.to_str().unwrap(),
            "unknown",
            "running binary differs",
        ),
    ] {
        let stdout = doctor(pid, program);
        assert!(
            stdout.contains(&format!("remote service program: {program}\n")),
            "{stdout}"
        );
        assert!(stdout.contains(&format!("remote: {feature}\n")), "{stdout}");
        assert!(
            stdout.contains(&format!("remote service: installed, {state}\n")),
            "{stdout}"
        );
        assert_eq!(
            stdout.lines().any(|line| line
                == "remote service repair: reinstall from source with --features remote"),
            feature == "disabled",
            "{stdout}"
        );
        assert_eq!(
            stdout
                .lines()
                .any(|line| line == "remote service repair: restart the service"),
            state == "running binary differs",
            "{stdout}"
        );
    }
}

#[test]
fn doctor_reports_manifest_binaries_once_with_versions() {
    let env = TestEnv::with_tmux(
        "niles-doctor-agents",
        "#!/bin/sh\nprintf 'tmux 3.5\\nsecond line\\n'\n",
    );
    write_workspace_manifest(
        &env.root,
        "pi:z-ai/glm-5.3-flash:high",
        "codex",
        "claude:opus:high",
        "claude",
    );
    write_executable(
        &env.bin.join("claude"),
        "#!/bin/sh\nprintf 'Claude CLI 1.0.0\\nsecond line\\n'\n",
    );
    write_executable(
        &env.bin.join("codex"),
        "#!/bin/sh\nprintf 'Codex CLI 1.0.0\\n'\n",
    );
    write_executable(&env.bin.join("pi"), "#!/bin/sh\nprintf 'Pi CLI 1.0.0\\n'\n");

    let output = env.run(&["doctor"]);

    assert_command_success("doctor agents", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains("tmux: tmux 3.5\n"), "{stdout}");
    assert!(stdout.contains(&format!(
        "agent claude: {} — Claude CLI 1.0.0; tested ",
        env.bin.join("claude").display()
    )));
    assert!(stdout.contains(&format!(
        "agent codex: {} — Codex CLI 1.0.0; tested ",
        env.bin.join("codex").display()
    )));
    assert!(stdout.contains("agent pi:"), "{stdout}");
    assert_eq!(stdout.matches("agent claude:").count(), 1);
    assert!(!stdout.contains("second line"));
}

#[test]
fn doctor_reports_missing_and_unavailable_builtin_versions() {
    let env = TestEnv::new("niles-doctor-unavailable");
    write_workspace_manifest(&env.root, "claude", "codex", "claude", "claude");
    fs::remove_file(env.bin.join("codex")).unwrap();
    write_executable(
        &env.bin.join("claude"),
        "#!/bin/sh\nprintf 'version failed\\n' >&2\nexit 1\n",
    );

    let output = env
        .niles(&env.root, &["doctor"])
        .env("PATH", &env.bin)
        .output()
        .unwrap();

    assert_success_contains(
        "doctor unavailable",
        &output,
        &format!(
            "agent claude: {} — version unavailable: version failed",
            env.bin.join("claude").display()
        ),
    );
    let stdout = stdout_of(&output);
    assert!(stdout.contains("agent codex: not found on PATH"));
    assert!(!stdout.contains("agent lead:"));
}

#[test]
fn doctor_dirty_source_tree_ignores_inherited_repository_routing() {
    let workspace = temp_workspace("niles-doctor-dirty-test");
    fs::create_dir_all(workspace.join("src")).unwrap();
    atomic_write(
        workspace.join("Cargo.toml"),
        r#"[package]
name = "niles"
version = "0.1.0"
edition = "2024"
"#,
    )
    .unwrap();
    atomic_write(workspace.join("src/main.rs"), "fn main() {}\n").unwrap();
    git(&workspace, &["init"]);
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "-m", "initial"]);
    let caller = temp_workspace("niles-doctor-caller");
    git(&workspace, &["clone", ".", caller.to_str().unwrap()]);
    atomic_write(
        workspace.join("src/main.rs"),
        "fn main() { println!(\"dirty\"); }\n",
    )
    .unwrap();

    let output = niles_bare(&workspace, &niles_home(&workspace))
        .arg("doctor")
        .env("GIT_DIR", caller.join(".git"))
        .env("GIT_WORK_TREE", &caller)
        .output()
        .unwrap();

    assert_command_success("doctor dirty", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains("dev_mode: yes"));
    assert!(stdout.contains("working_tree: dirty"));
    assert!(stdout.contains("stale: unknown (working tree dirty)"));
    assert!(!stdout.contains("stale: no"));
}
