#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{TestEnv, assert_command_success, state_dir, stdout_of, write_worker};
use std::{fs, process::Command};

#[test]
fn window_labels_need_metadata_but_never_logs_or_telemetry() {
    let env = TestEnv::new("niles-bar-labels");
    let registry = env.home.join(".niles/projects");
    fs::create_dir_all(&registry).unwrap();
    std::os::unix::fs::symlink(&env.root, registry.join("api")).unwrap();
    let worker = write_worker(&env.root, "parse", "api:w", None, b"");
    fs::remove_file(worker.join("status.log")).unwrap();
    fs::create_dir(worker.join("status.log")).unwrap();
    write_worker(&env.root, "lost", "api:gone", None, b"");
    let output = env
        .niles(&env.root, &["status", "sessions", "api"])
        .env("HOME", &env.home)
        .env(
            "TMUX_WINDOWS",
            "0\tniles\n1\tw\n2\tshell#{pane_id},#[fg=red]",
        )
        .output()
        .unwrap();
    assert_command_success("window labels", &output);
    let text = stdout_of(&output);
    assert!(text.contains("0:lead#[default] │ "), "{text}");
    assert!(text.contains("1:parse#[default] │ "), "{text}");
    assert!(
        text.contains("2:shell##{pane_id},##[fg=red]#[default]"),
        "{text}"
    );
    assert!(!text.contains("lost"), "{text}");
    assert_eq!(env.tmux_log().lines().count(), 1);
}

#[test]
fn cached_bar_output_selects_at_tmux_expansion_time() {
    let env = TestEnv::new("niles-bar-selection");
    let output = env
        .niles(&env.root, &["status", "sessions", "api", "99"])
        .env("HOME", &env.home)
        .env("TMUX_WINDOWS", "0\tfirst\n1\t#{pane_id},#[fg=red]")
        .output()
        .unwrap();
    assert_command_success("window format", &output);
    let format = stdout_of(&output);
    let registry = env.home.join(".niles/projects");
    fs::create_dir_all(&registry).unwrap();
    for name in ["api", "other"] {
        std::os::unix::fs::symlink(&env.root, registry.join(name)).unwrap();
    }
    let projects = env
        .niles(&env.root, &["status", "projects", "stale"])
        .env("HOME", &env.home)
        .env("TMUX_SESSION_EXISTS", "1")
        .env("TMUX_PROJECT_TAG", &env.root)
        .env("TMUX_WINDOWS", "niles\t0")
        .output()
        .unwrap();
    assert_command_success("project format", &projects);
    let projects = stdout_of(&projects);
    let socket = format!("niles-bar-{}", std::process::id());
    let tmux = |args: &[&str]| {
        let output = Command::new("tmux")
            .arg("-L")
            .arg(&socket)
            .args(args)
            .env("TMUX_TMPDIR", "/tmp")
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert_command_success("isolated tmux", &output);
        stdout_of(&output).trim_end().to_owned()
    };
    tmux(&[
        "-f",
        "/dev/null",
        "new-session",
        "-d",
        "-s",
        "api",
        "-n",
        "first",
        "sleep 300",
    ]);
    tmux(&["new-window", "-d", "-t", "api:1", "sleep 300"]);
    let first = tmux(&["display-message", "-p", "-t", "api:0", &format]);
    let second = tmux(&["display-message", "-p", "-t", "api:1", &format]);
    tmux(&["new-session", "-d", "-s", "other", "sleep 300"]);
    let api = tmux(&["display-message", "-p", "-t", "api:0", &projects]);
    let other = tmux(&["display-message", "-p", "-t", "other:0", &projects]);
    tmux(&["kill-server"]);
    assert!(
        api.starts_with("#[fg=") && api.ends_with("api#[default] │ other#[default]"),
        "{api}"
    );
    assert!(
        other.starts_with("api#[default] │ #[fg=") && other.ends_with("other#[default]"),
        "{other}"
    );
    assert!(first.starts_with("#[fg="), "{first}");
    assert!(first.contains(",bg="), "{first}");
    assert!(
        first.contains(",bold]0:first#[default] │ 1:#{pane_id},##[fg=red]"),
        "{first}"
    );
    assert!(second.starts_with("0:first#[default] │ #[fg="), "{second}");
    assert!(
        second.contains(",bold]1:#{pane_id},##[fg=red]#[default]"),
        "{second}"
    );
}

#[test]
fn projects_use_name_order_and_only_live_matching_membership() {
    let env = TestEnv::with_tmux(
        "niles-project-bar",
        r#"#!/bin/sh
[ "$1" != -u ] || shift
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1:$3" in
  has-session:=absent) printf "can't find session\n" >&2; exit 1 ;;
  display:*)
    if [ "$4" = '=wrong:' ]; then printf '/wrong/project\n';
    else printf '%s\n' "$PROJECT_PATH"; fi ;;
  list-windows:=dead) printf 'niles\t1\n' ;;
  list-windows:*) printf 'niles\t0\n' ;;
esac
"#,
    );
    let registry = env.home.join(".niles/projects");
    fs::create_dir_all(&registry).unwrap();
    for name in ["zeta", "wrong", "dead", "alpha", "absent"] {
        std::os::unix::fs::symlink(&env.root, registry.join(name)).unwrap();
    }
    std::os::unix::fs::symlink(env.root.join("missing"), registry.join("missing")).unwrap();
    let worker = state_dir(&env.root).join("worker/broken");
    fs::create_dir_all(&worker).unwrap();
    fs::write(worker.join("meta.json"), "invalid metadata").unwrap();
    let output = env
        .niles(&env.root, &["status", "projects"])
        .env("HOME", &env.home)
        .env("PROJECT_PATH", &env.root)
        .output()
        .unwrap();
    assert_command_success("project membership without telemetry", &output);
    let text = stdout_of(&output);
    let labels: Vec<_> = text
        .trim()
        .split(" │ ")
        .map(|part| part.rsplit_once('}').unwrap().1)
        .collect();
    assert_eq!(labels, ["alpha#[default]", "zeta#[default]"]);
    assert!(!env.tmux_log().contains("=missing"));
}
