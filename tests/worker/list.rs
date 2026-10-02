use super::support::*;

#[test]
fn workers_prints_complete_json_for_readable_and_unreadable_workers() {
    let env = TestEnv::new("niles-workers-json");
    let readable = write_worker(
        &env.root,
        "a-readable",
        "niles:niles-a-readable",
        Some("json"),
        b"working: first\ndone: complete\n",
    );
    fs::write(
        readable.join("checkin"),
        "deadline=2026-02-03T04:05:06Z\ndelay=300\nrecheck=backoff\narmed_len=0\n",
    )
    .unwrap();
    write_worker(&env.root, "b-clear", "old:niles-b-clear", None, b"");
    write_corrupt_worker_fixture(&env.root, "z-unreadable");

    let recovered = format!(
        "recovered:niles-renamed\t{}\tb-clear\t0",
        env.root.display()
    );
    let output = env
        .niles(&env.root, &["workers"])
        .env("TMUX_WINDOWS", "niles-a-readable\t0")
        .env("TMUX_MISSING_SESSION", "old")
        .env("TMUX_TAGGED_WINDOWS", recovered)
        .output()
        .unwrap();

    assert_command_success("workers", &output);
    let metadata_path = env.root.join(".niles/worker/z-unreadable/meta.json");
    let error = format!(
        "failed to parse {}: missing field `role` at line 4 column 1",
        metadata_path.display()
    );
    let error = serde_json::to_string(&error).unwrap();
    assert_eq!(
        stdout_of(&output),
        format!(
            "{{\"workers\":[{{\"id\":\"a-readable\",\"role\":\"worker\",\"agent\":\"codex\",\"task_label\":\"json\",\"started_at\":\"2026-01-02T03:04:05Z\",\"window\":{{\"state\":\"live\"}},\"wake\":\"pending\",\"last_status\":\"done: complete\",\"checkin\":{{\"deadline\":\"2026-02-03T04:05:06Z\"}},\"error\":null}},{{\"id\":\"b-clear\",\"role\":\"worker\",\"agent\":\"codex\",\"task_label\":null,\"started_at\":\"2026-01-02T03:04:05Z\",\"window\":{{\"state\":\"orphan-recovered\",\"target\":\"recovered:niles-renamed\"}},\"wake\":\"clear\",\"last_status\":null,\"checkin\":null,\"error\":null}},{{\"id\":\"z-unreadable\",\"role\":null,\"agent\":null,\"task_label\":null,\"started_at\":null,\"window\":null,\"wake\":null,\"last_status\":null,\"checkin\":null,\"error\":{error}}}]}}\n"
        )
    );
}
