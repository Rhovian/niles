use super::support::*;

#[test]
fn peek_quiet_disarms_only_after_a_successful_capture() {
    let env = TestEnv::new("niles-worker-peek-quiet");
    write_worker(&env.root, "worker", "niles:niles-worker", None, b"");
    let checkin = state_dir(&env.root).join("worker/worker/checkin");
    let arm = "deadline=2026-01-01T00:00:00Z\ndelay=300\nrecheck=backoff\narmed_len=0\n";
    let default_peek = env.niles(&env.root, &["peek", "worker"]).output().unwrap();
    assert_command_success("default peek", &default_peek);

    let full_history_peek = env
        .niles(&env.root, &["peek", "worker", "--lines", "0"])
        .output()
        .unwrap();
    assert_command_success("full-history peek", &full_history_peek);

    let log = env.tmux_log();
    assert!(
        log.lines()
            .any(|line| line == "capture-pane -p -t =niles:=niles-worker -S -200")
    );
    assert!(
        log.lines()
            .any(|line| line == "capture-pane -p -t =niles:=niles-worker -S -")
    );

    atomic_write(&checkin, arm).unwrap();

    let peek = env.run(&["peek", "worker"]);
    assert_command_success("peek without quiet", &peek);
    assert_eq!(fs::read_to_string(&checkin).unwrap(), arm);

    let quiet = env.run(&["peek", "worker", "--quiet"]);
    assert_command_success("peek quiet", &quiet);
    assert_eq!(stdout_of(&quiet), "pane output\n");
    assert_eq!(stderr_of(&quiet), "quiet: worker\n");
    assert!(!checkin.exists());

    let repeated = env.run(&["peek", "worker", "--quiet"]);
    assert_command_success("peek already quiet", &repeated);
    assert!(stderr_of(&repeated).contains("no check-in armed"));

    fs::create_dir(&checkin).unwrap();
    let failed_disarm = env.run(&["peek", "worker", "--quiet"]);
    assert!(!failed_disarm.status.success());
    assert_eq!(stdout_of(&failed_disarm), "pane output\n");
    assert!(stderr_of(&failed_disarm).contains("failed to remove"));
    fs::remove_dir(&checkin).unwrap();

    atomic_write(&checkin, arm).unwrap();
    write_executable(
        &env.bin.join("tmux"),
        "#!/bin/sh\nprintf 'capture failed' >&2\nexit 1\n",
    );
    let failed = env.run(&["peek", "worker", "--quiet"]);
    assert!(!failed.status.success());
    assert!(stderr_of(&failed).contains("capture failed"));
    assert_eq!(fs::read_to_string(&checkin).unwrap(), arm);
}
