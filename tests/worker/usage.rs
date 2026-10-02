use super::support::*;

#[test]
fn usage_reads_worker_and_live_lead_from_claude_store() {
    let env = TestEnv::new("niles-usage");
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
        .niles(&env.root, &["usage"])
        .env("HOME", &env.home)
        .env("TMUX_PANE", "%9")
        .output()
        .unwrap();
    assert_command_success("usage", &output);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = json["sessions"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], "impl");
    assert_eq!(rows[0]["role"], "worker");
    assert_eq!(rows[0]["agent"], "codex");
    assert_eq!(rows[0]["usage"]["input_tokens"], 7);
    assert_eq!(rows[1]["role"], "lead");
    assert_eq!(rows[1]["id"], "lead-one");
    assert_eq!(rows[1]["agent"], "claude");
    assert_eq!(rows[1]["usage"]["input_tokens"], 11);
}
