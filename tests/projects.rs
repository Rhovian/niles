#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::TestEnv;
use std::{
    fs,
    io::Write,
    os::fd::{FromRawFd, RawFd},
    process::{Command, Stdio},
};

fn terminal_input(command: &mut Command, input: &str) -> std::process::Output {
    let mut master: RawFd = -1;
    let mut slave: RawFd = -1;
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    command.stdin(unsafe { Stdio::from_raw_fd(slave) });
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = command.spawn().unwrap();
    let mut writer = unsafe { fs::File::from_raw_fd(master) };
    writer.write_all(input.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn create_registers_and_switches() {
    let env = TestEnv::new("niles-project-create");
    let project = env.root.join("scratch");
    fs::create_dir(&project).unwrap();
    let result = terminal_input(
        env.niles(&env.root, &[]).env("HOME", &env.home),
        &format!("n\n{}\nscratch\n", project.display()),
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        fs::read_link(env.home.join(".niles/projects/scratch")).unwrap(),
        project
    );
    let log = env.tmux_log();
    let steps = [
        "new-session -d -s scratch -c",
        "; set-option -t =scratch: @niles-project",
        "set-option -w -t =scratch:=niles remain-on-exit failed",
        "switch-client -t =scratch",
    ];
    let positions = steps.map(|step| log.find(step).unwrap());
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{log}");
}

#[test]
fn live_session_only_switches() {
    let env = TestEnv::new("niles-project-live");
    let dir = env.home.join(".niles/projects");
    fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(&env.root, dir.join("live")).unwrap();
    let result = terminal_input(
        env.niles(&env.root, &[])
            .env("HOME", &env.home)
            .env("TMUX_SESSION_EXISTS", "1")
            .env("TMUX_PROJECT_TAG", &env.root)
            .env("TMUX_WINDOWS", "niles\t0"),
        "1\n",
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let log = env.tmux_log();
    assert!(log.contains("switch-client -t =live"), "{log}");
    assert!(!log.contains("new-session"), "{log}");
}

#[test]
fn untagged_session_is_rejected() {
    let env = TestEnv::new("niles-project-collision");
    let dir = env.home.join(".niles/projects");
    fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(&env.root, dir.join("live")).unwrap();
    let result = terminal_input(
        env.niles(&env.root, &[])
            .env("HOME", &env.home)
            .env("TMUX_SESSION_EXISTS", "1"),
        "1\n",
    );
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("isn't niles' session"));
}
