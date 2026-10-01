use super::support::*;

#[test]
fn worker_close_recovers_renamed_orphan_by_matching_tags() {
    let env = TestEnv::new("niles-close-recovered");
    write_worker(
        &env.root,
        "auth-fix",
        "old:niles-auth-fix",
        None,
        b"working: close requested",
    );

    let tagged = format!("new:niles-renamed\t{}\tauth-fix\t0", env.root.display());
    let close = env
        .niles(&env.root, &["close", "auth-fix"])
        .env("TMUX_MISSING_SESSION", "old")
        .env("TMUX_TAGGED_WINDOWS", tagged)
        .output()
        .unwrap();
    assert_command_success("recovered orphan close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(
        stdout.contains("window state: orphan-recovered:old:niles-auth-fix->new:niles-renamed")
    );

    let log = env.tmux_log();
    assert!(log.contains("capture-pane -p -t =new:=niles-renamed -S -2000"));
    assert!(log.contains("kill-window -t =new:=niles-renamed"));
    assert_archived_with_closed_sentinel(&env.root, "auth-fix");
}

#[test]
fn worker_close_ignores_same_id_tag_from_other_workspace() {
    let env = TestEnv::new("niles-close-other-workspace");
    let other_workspace = temp_workspace("niles-close-other-project");
    write_worker(
        &env.root,
        "auth-fix",
        "old:niles-auth-fix",
        None,
        b"working: close requested",
    );

    let tagged = format!(
        "other:niles-auth-fix\t{}\tauth-fix\t0",
        other_workspace.display()
    );
    let close = env
        .niles(&env.root, &["close", "auth-fix"])
        .env("TMUX_MISSING_SESSION", "old")
        .env("TMUX_TAGGED_WINDOWS", tagged)
        .output()
        .unwrap();
    assert_command_success("cross-workspace tag close", &close);
    assert!(String::from_utf8_lossy(&close.stdout).contains("window state: orphan-gone"));

    let log = env.tmux_log();
    assert!(!log.contains("kill-window"));
    assert_archived_with_closed_sentinel(&env.root, "auth-fix");
    fs::remove_dir_all(other_workspace).unwrap();
}

#[test]
fn worker_close_multiple_tag_matches_reaps_without_kill() {
    let env = TestEnv::new("niles-close-multiple-tags");
    write_worker(
        &env.root,
        "auth-fix",
        "old:niles-auth-fix",
        None,
        b"working: close requested",
    );

    let tagged = format!(
        "one:niles-auth-fix\t{}\tauth-fix\t0\ntwo:niles-auth-fix\t{}\tauth-fix\t0",
        env.root.display(),
        env.root.display()
    );
    let close = env
        .niles(&env.root, &["close", "auth-fix"])
        .env("TMUX_MISSING_SESSION", "old")
        .env("TMUX_TAGGED_WINDOWS", tagged)
        .output()
        .unwrap();
    assert_command_success("multiple tagged orphan close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("window state: unknown:multiple tmux windows carry worker tags"));

    let log = env.tmux_log();
    assert!(!log.contains("kill-window"));
    assert_archived_with_closed_sentinel(&env.root, "auth-fix");
}

#[test]
fn worker_close_recovers_window_missing_by_matching_tags() {
    let env = TestEnv::new("niles-close-window-missing-recovered");
    write_worker(
        &env.root,
        "auth-fix",
        "home:niles-auth-fix",
        None,
        b"working: close requested",
    );

    let tagged = format!("other:niles-renamed\t{}\tauth-fix\t0", env.root.display());
    let close = env
        .niles(&env.root, &["close", "auth-fix"])
        .env("TMUX_TAGGED_WINDOWS", tagged)
        .output()
        .unwrap();
    assert_command_success("window-missing recovered close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(
        stdout.contains("window state: orphan-recovered:home:niles-auth-fix->other:niles-renamed")
    );

    let log = env.tmux_log();
    assert!(log.contains("list-windows -t =home -F #{window_name}"));
    assert!(log.contains("capture-pane -p -t =other:=niles-renamed -S -2000"));
    assert!(log.contains("kill-window -t =other:=niles-renamed"));
    assert_archived_with_closed_sentinel(&env.root, "auth-fix");
}

#[test]
fn worker_close_window_missing_without_tag_is_window_dead() {
    let env = TestEnv::new("niles-close-window-missing-dead");
    write_worker(
        &env.root,
        "auth-fix",
        "home:niles-auth-fix",
        None,
        b"working: close requested",
    );

    let close = env
        .niles(&env.root, &["close", "auth-fix"])
        .output()
        .unwrap();
    assert_command_success("window-missing dead close", &close);
    assert!(String::from_utf8_lossy(&close.stdout).contains("window state: window-dead"));

    let log = env.tmux_log();
    assert!(!log.contains("kill-window"));
    assert_archived_with_closed_sentinel(&env.root, "auth-fix");
}

#[test]
fn worker_close_reports_legacy_candidate_without_auto_kill() {
    let env = TestEnv::new("niles-close-legacy-candidate");
    write_worker(
        &env.root,
        "auth-fix",
        "old:niles-auth-fix",
        None,
        b"working: close requested",
    );

    let close = env
        .niles(&env.root, &["close", "auth-fix"])
        .env("TMUX_MISSING_SESSION", "old")
        .env("TMUX_TAGGED_WINDOWS", "other:niles-auth-fix\t\t\t0")
        .output()
        .unwrap();
    assert_command_success("legacy candidate close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("window state: orphan-legacy-candidate:other:niles-auth-fix"));
    assert!(stdout.contains("manual_close: tmux kill-window -t other:niles-auth-fix"));

    let log = env.tmux_log();
    assert!(!log.contains("kill-window"));
    assert_archived_with_closed_sentinel(&env.root, "auth-fix");
}

#[test]
fn worker_close_reaps_unparseable_meta_window_as_unknown() {
    let env = TestEnv::new("niles-close-invalid-window");
    write_worker(
        &env.root,
        "auth-fix",
        "niles-auth-fix",
        None,
        b"working: close requested",
    );

    let close = env
        .niles(&env.root, &["close", "auth-fix"])
        .output()
        .unwrap();
    assert_command_success("invalid-window close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(
        stdout.contains(
            "window state: unknown:worker auth-fix metadata has invalid tmux window target"
        )
    );

    assert!(!env.tmux_log.exists());
    assert_archived_with_closed_sentinel(&env.root, "auth-fix");
}

#[test]
fn worker_close_reaps_session_gone_orphan_without_tmux_error() {
    let env = TestEnv::new("niles-close-gone");
    write_worker(
        &env.root,
        "auth-fix",
        "old:niles-auth-fix",
        None,
        b"working: close requested",
    );

    let close = env
        .niles(&env.root, &["close", "auth-fix"])
        .env("TMUX_MISSING_SESSION", "old")
        .output()
        .unwrap();
    assert_command_success("gone orphan close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("window state: orphan-gone"));
    assert!(!stdout.contains("can't find session"));

    let log = env.tmux_log();
    assert!(!log.contains("kill-window"));
    assert_archived_with_closed_sentinel(&env.root, "auth-fix");
}
