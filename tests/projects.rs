#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{TestEnv, assert_command_success as assert_ok, stdout_of};
use std::{fs, process::Command};

#[test]
fn bare_niles_creates_the_home_session_and_switches() {
    let env = TestEnv::with_tmux(
        "niles-home-create",
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  has-session) printf "can't find session\n" >&2; exit 1 ;;
  display-message) printf '160 45\n' ;;
  display) if [ "$5" = '#{pane_id}' ]; then printf '%%0\n'; else printf '/tmp/help-test.sock\n'; fi ;;
  split-window) printf '%%2\t/dev/ttys002\n' ;;
esac
"#,
    );
    let result = env.niles(&env.root, &[]).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let log = env.tmux_log();
    let cwd = env.root.canonicalize().unwrap();
    let steps = [
        format!("new-session -d -x 160 -y 45 -s niles+home -c {}", cwd.display()),
        "explorer ; set-option -t =niles+home: status off ; set-option -t =niles+home: mouse on ; set-option -p -t =niles+home: remain-on-exit failed".to_owned(),
        "new-window -d -t =niles+panels: -n help".to_owned(),
        "panel help".to_owned(),
        "split-window -h -l 75% -t =niles+home:".to_owned(),
        "switch-client -c /dev/ttys002 -t =niles+panels:=help".to_owned(),
        "select-pane -t %2".to_owned(),
        "select-pane -t %0".to_owned(),
        "switch-client -t =niles+home:".to_owned(),
    ];
    let positions = steps.map(|step| log.find(&step).unwrap_or_else(|| panic!("{step}\n{log}")));
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{log}");
}

#[test]
fn existing_home_session_only_switches() {
    let env = TestEnv::new("niles-home-live");
    let result = env
        .niles(&env.root, &[])
        .env("TMUX_SESSION_EXISTS", "1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let log = env.tmux_log();
    assert!(log.contains("switch-client -t =niles+home:"), "{log}");
    assert!(!log.contains("new-session"), "{log}");
}

#[test]
fn home_bindings_install_on_a_real_server() {
    let found = Command::new("which").arg("tmux").output().unwrap();
    let real_tmux = String::from_utf8(found.stdout).unwrap().trim().to_owned();
    let wrapper = r#"#!/bin/sh
[ "$1" = switch-client ] || exec "$TMUX_REAL" -S "$TMUX_SOCKET" "$@"
"#;
    let env = TestEnv::with_tmux("niles-bindings", wrapper);
    let socket = env.root.join("tmux.sock");
    let tmux = |args: &[&str]| {
        let mut command = Command::new(&real_tmux);
        command.args(["-f", "/dev/null", "-S", socket.to_str().unwrap()]);
        command.args(args).output().unwrap()
    };
    let start = ["new-session", "-d", "-s", "niles+home"];
    assert_ok("start tmux", &tmux(&start));
    fs::create_dir_all(env.home.join(".niles")).unwrap();
    let config = "tmux: {bindings: true}";
    fs::write(env.home.join(".niles/config.yaml"), config).unwrap();
    let mut niles = env.niles(&env.root, &[]);
    niles.envs([
        ("HOME", env.home.to_str().unwrap()),
        ("TMUX_REAL", &real_tmux),
        ("TMUX_SOCKET", socket.to_str().unwrap()),
    ]);
    let result = niles.output().unwrap();
    let installed = tmux(&["list-keys", "-T", "root"]);
    assert_ok("stop tmux", &tmux(&["kill-server"]));
    assert_ok("bare niles", &result);
    let keys = stdout_of(&installed);
    let popup = format!("display-popup -E \"'{}'\"", env!("CARGO_BIN_EXE_niles"));
    let line = |key| {
        keys.lines()
            .find(|l| l.split_whitespace().nth(3) == Some(key))
            .unwrap_or_else(|| panic!("missing {key}: {keys}"))
    };
    assert!(line("M-n").contains(&popup), "{keys}");
    for (meta, plain, pass) in [
        ("M-[", "'['", "'M-['"),
        ("M-]", "']'", "'M-]'"),
        ("\"M-;\"", "';'", "'M-;'"),
        ("\"M-'\"", "''\\\\'''", "'M-'\\\\'''"),
    ] {
        let line = line(meta);
        assert!(line.contains("#{==:#{session_name},niles+home}"), "{line}");
        let send = format!("send-keys -t '=niles+home:{{start}}.{{top-left}}' {plain}");
        assert!(line.contains(&send), "{line}");
        assert!(line.contains(&format!("send-keys {pass}")), "{line}");
    }
}
