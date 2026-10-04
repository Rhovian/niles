#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::TestEnv;

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
        "explorer ; set-option -t =niles+home: status off ; set-option -p -t =niles+home: remain-on-exit failed".to_owned(),
        format!("; split-window -d -f -v -l 2 -t =niles+home: -c {}", cwd.display()),
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
