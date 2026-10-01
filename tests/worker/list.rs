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
        "deadline=2026-02-03T04:05:06Z\ndelay=300\nstep=300\nrecheck=backoff\narmed_len=0\n",
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
            "{{\"workers\":[{{\"id\":\"a-readable\",\"role\":\"worker\",\"agent\":\"codex\",\"task_label\":\"json\",\"started_at\":\"2026-01-02T03:04:05Z\",\"window\":{{\"state\":\"live\"}},\"wake\":\"pending\",\"last_status\":\"done: complete\",\"checkin\":{{\"deadline\":\"2026-02-03T04:05:06Z\"}},\"error\":null,\"usage\":null}},{{\"id\":\"b-clear\",\"role\":\"worker\",\"agent\":\"codex\",\"task_label\":null,\"started_at\":\"2026-01-02T03:04:05Z\",\"window\":{{\"state\":\"orphan-recovered\",\"target\":\"recovered:niles-renamed\"}},\"wake\":\"clear\",\"last_status\":null,\"checkin\":null,\"error\":null,\"usage\":null}},{{\"id\":\"z-unreadable\",\"role\":null,\"agent\":null,\"task_label\":null,\"started_at\":null,\"window\":null,\"wake\":null,\"last_status\":null,\"checkin\":null,\"error\":{error},\"usage\":null}}]}}\n"
        )
    );
}

#[test]
fn workers_reads_worker_and_live_lead_from_claude_store() {
    let env = TestEnv::new("niles-workers-usage");
    let worker = write_worker(&env.root, "impl", "niles:niles-impl", None, b"");
    let path = worker.join("meta.json");
    let mut meta: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    meta["session_link"] = serde_json::json!({"family":"claude","session_id":"worker-session"});
    fs::write(&path, serde_json::to_vec(&meta).unwrap()).unwrap();
    let lead = env.root.join(".niles/sessions/lead-one");
    fs::create_dir_all(&lead).unwrap();
    fs::write(
        lead.join("session.json"),
        serde_json::to_vec(&serde_json::json!({
            "id":"lead-one","agent":"claude","agent_family":null,"model":null,"effort":null,
            "created_at":"2026-01-02T03:04:05Z","workspace":env.root,"brief":lead.join("lead.md"),
            "lead_pane":"%9","session_link":{"family":"claude","session_id":"lead-session"}
        }))
        .unwrap(),
    )
    .unwrap();
    let store = env.home.join(".claude/projects/project");
    fs::create_dir_all(&store).unwrap();
    for (id, tokens) in [("worker-session", 7), ("lead-session", 11)] {
        fs::write(store.join(format!("{id}.jsonl")), format!(
            "{{\"type\":\"assistant\",\"timestamp\":\"2026-01-02T04:00:00Z\",\"message\":{{\"id\":\"{id}\",\"model\":\"opus\",\"usage\":{{\"input_tokens\":{tokens},\"output_tokens\":2,\"cache_read_input_tokens\":3,\"cache_creation_input_tokens\":4}}}}}}\n"
        )).unwrap();
    }
    let output = env
        .niles(&env.root, &["workers"])
        .env("HOME", &env.home)
        .env("TMUX_PANE", "%9")
        .output()
        .unwrap();
    assert_command_success("workers usage", &output);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = json["workers"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["usage"]["input_tokens"], 7);
    assert_eq!(rows[1]["role"], "lead");
    assert_eq!(rows[1]["usage"]["input_tokens"], 11);
}
