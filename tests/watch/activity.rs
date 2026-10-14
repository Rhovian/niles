use super::*;

#[test]
fn session_activity_lifecycle_requires_a_record_and_a_live_pane() {
    let fixture = fixture("niles-session-lifecycle");
    assert_command_success("spawn", &fixture.spawn("impl"));
    let meta: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.worker_file("impl", "meta.json")).unwrap())
            .unwrap();
    let session = meta["session_link"]["session_id"].as_str().unwrap();
    let workers = |dead: bool| {
        let output = fixture
            .env
            .niles(&fixture.env.root, &["workers"])
            .env("HOME", &fixture.env.home)
            .env(
                "TMUX_WINDOWS",
                if dead {
                    "niles-impl\t1"
                } else {
                    "niles-impl\t0"
                },
            )
            .output()
            .unwrap();
        assert_command_success("workers", &output);
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        value["workers"][0]["state"].as_str().unwrap().to_owned()
    };
    assert_eq!(workers(false), "starting");
    let store = fixture.env.home.join(".claude/projects/project");
    fs::create_dir_all(&store).unwrap();
    let path = store.join(format!("{session}.jsonl"));
    fs::write(&path, "").unwrap();
    assert_eq!(workers(false), "starting");
    let turn = serde_json::json!({"type":"assistant","timestamp":chrono::Utc::now(),"message":{
        "id":"working","usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0},"stop_reason":"tool_use","content":[]}});
    fs::write(&path, format!("{turn}\n")).unwrap();
    assert_eq!(workers(false), "working");
    assert_eq!(workers(true), "waiting");
}

#[test]
fn session_activity_missing_after_one_interval_reports_unknown_once() {
    let fixture = fixture("niles-session-unknown");
    assert_command_success("spawn", &fixture.spawn("impl"));
    let path = fixture.worker_file("impl", "meta.json");
    let mut meta: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let deadline = chrono::Utc::now() - chrono::TimeDelta::seconds(1);
    meta["created_at"] = (deadline - chrono::TimeDelta::seconds(300))
        .to_rfc3339()
        .into();
    meta["starting_until"] = deadline.to_rfc3339().into();
    fs::write(path, meta.to_string()).unwrap();
    fs::write(
        fixture.checkin_path("impl"),
        format!(
            "deadline={}\ndelay=300\nrecheck=backoff\narmed_len=0\n",
            deadline.to_rfc3339()
        ),
    )
    .unwrap();
    write_executable(&fixture.env.bin.join("claude"), "#!/bin/sh\nsleep 3\n");
    let output = fixture
        .env
        .niles(&fixture.env.root, &["lead"])
        .env("HOME", &fixture.env.home)
        .env("TMUX_PANE", "%9")
        .env("TMUX_PANE_FILE", &fixture.pane_file)
        .output()
        .unwrap();
    assert_command_success("lead", &output);
    assert_eq!(
        fs::read_to_string(fixture.status_log("impl")).unwrap(),
        "blocked: session state unknown (claude:sonnet): session activity unavailable after one check-in interval\n"
    );
    assert!(!fixture.checkin_path("impl").exists());
    let log = fixture.env.tmux_log();
    assert_eq!(
        log.matches("niles: impl reported (blocked) — check workers")
            .count(),
        1,
        "{log}"
    );
    assert!(!log.contains("no report from impl"), "{log}");
}
