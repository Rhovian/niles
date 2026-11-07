use std::process::Command;

use super::*;

#[test]
fn prefixes_follow_the_view_and_global_configuration() {
    if env::var_os("NILES_HOME_TMUX_TEST").is_none() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("tmux.sock");
        let server = |args: &[&str]| {
            Command::new("tmux")
                .args(["-f", "/dev/null", "-S"])
                .arg(&socket)
                .args(args)
                .output()
                .unwrap()
        };
        assert!(
            server(&["new-session", "-d", "-s", HOME_SESSION])
                .status
                .success()
        );
        // Give the child its own server without changing this test process's environment.
        let output = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "tmux::home::tests::prefixes_follow_the_view_and_global_configuration",
                "--nocapture",
            ])
            .env("TMUX", format!("{},0,0", socket.display()))
            .env("NILES_HOME_TMUX_TEST", "1")
            .output()
            .unwrap();
        assert!(server(&["kill-server"]).status.success());
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let home = SessionName::new(HOME_SESSION).unwrap();
    let explorer = TmuxTarget::pane(&explorer_pane()).unwrap();
    let prefixes = || {
        query(&[
            "display",
            "-p",
            "-t",
            &explorer_pane(),
            "#{prefix} #{prefix2}",
        ])
        .unwrap()
    };
    run(&[
        "set", "-g", "prefix", "C-a", ";", "set", "-g", "prefix2", "C-q",
    ])
    .unwrap();
    run(&["new-session", "-d", "-s", "project", "-n", "niles"]).unwrap();
    let project = WindowTarget::new(SessionName::new("project").unwrap(), "niles").unwrap();
    // This calls the real split/tag/select path; removing its refresh leaves prefixes enabled.
    configure_view_prefix(&home).unwrap();
    show_in_view(&explorer, &project).unwrap();
    assert_eq!(prefixes().trim(), "None None");
    // Installing from another window must still cover the explorer/view window.
    run(&["new-window", "-t", "=niles+home:", "-n", "other"]).unwrap();
    run(&[
        "set-hook",
        "-u",
        "-t",
        &explorer_pane(),
        "window-pane-changed",
    ])
    .unwrap();
    configure_view_prefix(&home).unwrap();
    let hooks = query(&["show-hooks", "-w", "-t", &explorer_pane()]).unwrap();
    assert!(hooks.contains("window-pane-changed"), "{hooks}");
    run(&["select-window", "-t", &explorer_pane()]).unwrap();
    focus_explorer().unwrap();
    assert_eq!(prefixes().trim(), "C-a C-q");
    let view = view_pane(&explorer).unwrap().unwrap();
    run(&["select-pane", "-t", &view.id]).unwrap();
    assert_eq!(prefixes().trim(), "None None");
    // Configuration must initialize routing even before a pane-change hook has fired.
    run(&[
        "set-hook",
        "-u",
        "-t",
        &explorer_pane(),
        "window-pane-changed",
    ])
    .unwrap();
    run(&["set", "-u", "-t", "=niles+home:", "prefix"]).unwrap();
    configure_view_prefix(&home).unwrap();
    assert_eq!(prefixes().trim(), "None None");
    run(&["set", "-g", "prefix", "C-x"]).unwrap();
    focus_explorer().unwrap();
    assert_eq!(prefixes().trim(), "C-x C-q");
    run(&["select-pane", "-t", &view.id]).unwrap();
    run(&["kill-pane", "-t", &view.id]).unwrap();
    assert_eq!(prefixes().trim(), "C-x C-q");
}
