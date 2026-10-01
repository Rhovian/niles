use super::support::*;

/// A bare peek is read into the lead's context, so its default is bounded; the full scrollback is
/// available but has to be asked for with `--lines 0`.
#[test]
fn peek_defaults_to_a_glance_and_zero_lines_captures_full_history() {
    let env = TestEnv::new("niles-worker-peek-deep");
    write_worker(
        &env.root,
        "auth-fix",
        "niles:niles-auth-fix",
        None,
        b"working: inspect pane",
    );

    let default_peek = env
        .niles(&env.root, &["peek", "auth-fix"])
        .output()
        .unwrap();
    assert_command_success("default peek", &default_peek);

    let full_history_peek = env
        .niles(&env.root, &["peek", "auth-fix", "--lines", "0"])
        .output()
        .unwrap();
    assert_command_success("full-history peek", &full_history_peek);

    let log = env.tmux_log();
    assert!(
        log.lines()
            .any(|line| line == "capture-pane -p -t =niles:=niles-auth-fix -S -200")
    );
    assert!(
        log.lines()
            .any(|line| line == "capture-pane -p -t =niles:=niles-auth-fix -S -")
    );
}
