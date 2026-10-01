use super::support::*;

#[test]
fn worker_close_recovers_renamed_orphan_by_matching_tags() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-recovered");
    let home = niles_home(&workspace);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_orphan_recovery_tmux(&bin, "old");
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "old:niles-auth-fix",
    );

    let tagged = format!("new:niles-renamed\t{}\tauth-fix\t0", workspace.display());
    let close = Command::new(niles)
        .args(["close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_TAGGED_WINDOWS", tagged)
        .output()
        .unwrap();
    assert_command_success("recovered orphan close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(
        stdout.contains("window state: orphan-recovered:old:niles-auth-fix->new:niles-renamed")
    );

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("capture-pane -p -t =new:=niles-renamed -S -2000"));
    assert!(log.contains("kill-window -t =new:=niles-renamed"));
    let archive_dir = latest_archive_dir(&workspace, "auth-fix");
    assert!(
        fs::read_to_string(archive_dir.join("status.log"))
            .unwrap()
            .contains("closed: auth-fix")
    );
}

#[test]
fn worker_close_ignores_same_id_tag_from_other_workspace() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-other-workspace");
    let other_workspace = temp_workspace("niles-close-other-project");
    let home = niles_home(&workspace);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_orphan_recovery_tmux(&bin, "old");
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "old:niles-auth-fix",
    );

    let tagged = format!(
        "other:niles-auth-fix\t{}\tauth-fix\t0",
        other_workspace.display()
    );
    let close = Command::new(niles)
        .args(["close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_TAGGED_WINDOWS", tagged)
        .output()
        .unwrap();
    assert_command_success("cross-workspace tag close", &close);
    assert!(String::from_utf8_lossy(&close.stdout).contains("window state: orphan-gone"));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(!log.contains("kill-window"));
    assert_archived_with_closed_sentinel(&workspace, "auth-fix");
    fs::remove_dir_all(other_workspace).unwrap();
}

#[test]
fn worker_close_multiple_tag_matches_reaps_without_kill() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-multiple-tags");
    let home = niles_home(&workspace);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_orphan_recovery_tmux(&bin, "old");
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "old:niles-auth-fix",
    );

    let tagged = format!(
        "one:niles-auth-fix\t{}\tauth-fix\t0\ntwo:niles-auth-fix\t{}\tauth-fix\t0",
        workspace.display(),
        workspace.display()
    );
    let close = Command::new(niles)
        .args(["close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_TAGGED_WINDOWS", tagged)
        .output()
        .unwrap();
    assert_command_success("multiple tagged orphan close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("window state: unknown:multiple tmux windows carry worker tags"));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(!log.contains("kill-window"));
    assert_archived_with_closed_sentinel(&workspace, "auth-fix");
}

#[test]
fn worker_close_recovers_window_missing_by_matching_tags() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-window-missing-recovered");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "home:niles-auth-fix",
    );

    let tagged = format!("other:niles-renamed\t{}\tauth-fix\t0", workspace.display());
    let close = Command::new(niles)
        .args(["close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_TAGGED_WINDOWS", tagged)
        .output()
        .unwrap();
    assert_command_success("window-missing recovered close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(
        stdout.contains("window state: orphan-recovered:home:niles-auth-fix->other:niles-renamed")
    );

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("list-windows -t =home -F #{window_name}"));
    assert!(log.contains("capture-pane -p -t =other:=niles-renamed -S -2000"));
    assert!(log.contains("kill-window -t =other:=niles-renamed"));
    assert_archived_with_closed_sentinel(&workspace, "auth-fix");
}

#[test]
fn worker_close_window_missing_without_tag_is_window_dead() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-window-missing-dead");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "home:niles-auth-fix",
    );

    let close = Command::new(niles)
        .args(["close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .output()
        .unwrap();
    assert_command_success("window-missing dead close", &close);
    assert!(String::from_utf8_lossy(&close.stdout).contains("window state: window-dead"));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(!log.contains("kill-window"));
    assert_archived_with_closed_sentinel(&workspace, "auth-fix");
}

#[test]
fn worker_close_reports_legacy_candidate_without_auto_kill() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-legacy-candidate");
    let home = niles_home(&workspace);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_orphan_recovery_tmux(&bin, "old");
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "old:niles-auth-fix",
    );

    let close = Command::new(niles)
        .args(["close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_TAGGED_WINDOWS", "other:niles-auth-fix\t\t\t0")
        .output()
        .unwrap();
    assert_command_success("legacy candidate close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("window state: orphan-legacy-candidate:other:niles-auth-fix"));
    assert!(stdout.contains("manual_close: tmux kill-window -t other:niles-auth-fix"));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(!log.contains("kill-window"));
    assert_archived_with_closed_sentinel(&workspace, "auth-fix");
}

#[test]
fn worker_close_reaps_unparseable_meta_window_as_unknown() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-invalid-window");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "niles-auth-fix",
    );

    let close = Command::new(niles)
        .args(["close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .output()
        .unwrap();
    assert_command_success("invalid-window close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(
        stdout.contains(
            "window state: unknown:worker auth-fix metadata has invalid tmux window target"
        )
    );

    assert!(!tmux_log.exists());
    assert_archived_with_closed_sentinel(&workspace, "auth-fix");
}

#[test]
fn worker_close_reaps_session_gone_orphan_without_tmux_error() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-gone");
    let home = niles_home(&workspace);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_orphan_recovery_tmux(&bin, "old");
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "old:niles-auth-fix",
    );

    let close = Command::new(niles)
        .args(["close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .output()
        .unwrap();
    assert_command_success("gone orphan close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("window state: orphan-gone"));
    assert!(!stdout.contains("can't find session"));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(!log.contains("kill-window"));
    let archive_dir = latest_archive_dir(&workspace, "auth-fix");
    assert!(
        fs::read_to_string(archive_dir.join("status.log"))
            .unwrap()
            .contains("closed: auth-fix")
    );
}

#[test]
fn worker_close_reaps_current_schema_legacy_missing_session_meta() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-aquila");
    let home = niles_home(&workspace);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_orphan_recovery_tmux(&bin, "aquila");
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "aquila:niles-auth-fix",
    );

    let close = Command::new(niles)
        .args(["close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .output()
        .unwrap();
    assert_command_success("back-compat missing session close", &close);
    assert!(String::from_utf8_lossy(&close.stdout).contains("window state: orphan-gone"));
    assert!(!workspace.join(".niles/worker/auth-fix").exists());
    assert!(
        latest_archive_dir(&workspace, "auth-fix")
            .join("meta.json")
            .is_file()
    );
}
