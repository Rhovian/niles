#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::TestEnv;

#[test]
fn bare_niles_creates_the_home_session_and_switches() {
    let env = TestEnv::new("niles-home-create");
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
        format!("; split-window -d -f -v -l 8 -t =niles+home: -c {}", cwd.display()),
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
