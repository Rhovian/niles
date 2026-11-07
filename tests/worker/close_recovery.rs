use super::support::*;

#[test]
fn worker_close_recovers_or_reaps_each_window_state() {
    for (label, window, missing, tagged, expected, log_checks) in [
        (
            "worker_close_recovers_renamed_orphan_by_matching_tags",
            "old:niles-auth-fix",
            "old",
            "new:niles-renamed\t{root}\tauth-fix\t0",
            "window state: orphan-recovered:old:niles-auth-fix->new:niles-renamed",
            Some(&[
                ("capture-pane -p -t =new:=niles-renamed -S -2000", true),
                ("kill-window -t =new:=niles-renamed", true),
            ] as &[(&str, bool)]),
        ),
        (
            "worker_close_ignores_same_id_tag_from_other_workspace",
            "old:niles-auth-fix",
            "old",
            "other:niles-auth-fix\t{other}\tauth-fix\t0",
            "window state: orphan-gone",
            Some(&[("kill-window", false)]),
        ),
        (
            "worker_close_multiple_tag_matches_reaps_without_kill",
            "old:niles-auth-fix",
            "old",
            "one:niles-auth-fix\t{root}\tauth-fix\t0\ntwo:niles-auth-fix\t{root}\tauth-fix\t0",
            "window state: unknown:multiple tmux windows carry worker tags",
            Some(&[("kill-window", false)]),
        ),
        (
            "worker_close_recovers_window_missing_by_matching_tags",
            "home:niles-auth-fix",
            "",
            "other:niles-renamed\t{root}\tauth-fix\t0",
            "window state: orphan-recovered:home:niles-auth-fix->other:niles-renamed",
            Some(&[
                ("list-windows -t =home -F #{window_name}", true),
                ("capture-pane -p -t =other:=niles-renamed -S -2000", true),
                ("kill-window -t =other:=niles-renamed", true),
            ]),
        ),
        (
            "worker_close_window_missing_without_tag_is_window_dead",
            "home:niles-auth-fix",
            "",
            "",
            "window state: window-dead",
            Some(&[("kill-window", false)]),
        ),
        (
            "worker_close_reaps_unparseable_meta_window_as_unknown",
            "niles-auth-fix",
            "",
            "",
            "window state: unknown:worker auth-fix metadata has invalid tmux window target",
            None,
        ),
        (
            "worker_close_reaps_session_gone_orphan_without_tmux_error",
            "old:niles-auth-fix",
            "old",
            "",
            "window state: orphan-gone",
            Some(&[("kill-window", false)]),
        ),
    ] {
        let env = TestEnv::new(label);
        let other = env.root.join("other-project");
        fs::create_dir(&other).unwrap();
        let tagged = tagged
            .replace("{root}", env.root.to_str().unwrap())
            .replace("{other}", other.to_str().unwrap());
        let close = close_auth_fix(
            &env,
            window,
            b"working: close requested",
            &[
                ("TMUX_MISSING_SESSION", missing),
                ("TMUX_TAGGED_WINDOWS", &tagged),
            ],
        );
        assert_success_contains(label, &close, expected);
        assert!(!stdout_of(&close).contains("can't find session"), "{label}");
        if let Some(checks) = log_checks {
            let log = env.tmux_log();
            for (needle, present) in checks {
                assert_eq!(log.contains(needle), *present, "{label}: {needle}: {log}");
            }
        } else {
            assert!(!env.tmux_log.exists(), "{label}");
        }
        assert_archived_with_closed_sentinel(&env.root, "auth-fix");
    }
}
