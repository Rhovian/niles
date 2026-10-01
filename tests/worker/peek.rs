use super::support::*;

/// A bare peek is read into the lead's context, so its default is bounded; the full scrollback is
/// available but has to be asked for with `--lines 0`.
#[test]
fn peek_defaults_to_a_glance_and_zero_lines_captures_full_history() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-peek-deep");
    let home = niles_home(&workspace);
    write_worker_fixture(&workspace, "auth-fix", "working: inspect pane");

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_executable(
        &bin.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  display-message) printf 'niles-test-session\n'; exit 0 ;;
  capture-pane) printf 'pane output\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("PATH must be set in the test environment")
    );

    let default_peek = Command::new(niles)
        .args(["peek", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .output()
        .unwrap();
    assert_command_success("default peek", &default_peek);

    let full_history_peek = Command::new(niles)
        .args(["peek", "auth-fix", "--lines", "0"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .output()
        .unwrap();
    assert_command_success("full-history peek", &full_history_peek);

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(
        log.lines()
            .any(|line| line == "capture-pane -p -t =niles:=niles-auth-fix -S -200")
    );
    assert!(
        log.lines()
            .any(|line| line == "capture-pane -p -t =niles:=niles-auth-fix -S -")
    );
}
