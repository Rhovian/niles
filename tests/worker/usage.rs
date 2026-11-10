use super::support::*;

#[test]
fn usage_reads_worker_and_live_lead_from_claude_store() {
    let env = TestEnv::new("niles-usage");
    let worker = write_worker(&env.root, "impl", "niles:niles-impl", None, b"");
    edit_json(worker.join("meta.json"), |meta| {
        meta.as_object_mut().unwrap().extend(serde_json::json!({"skill":"lead","session_link":{"family":"claude","session_id":"worker-session"}}).as_object().unwrap().clone());
    });
    let lead = state_dir(&env.root).join("sessions/lead-one");
    fs::create_dir_all(&lead).unwrap();
    atomic_write(
        lead.join("session.json"),
        serde_json::to_vec(&serde_json::json!({
            "id":"lead-one","entry_skill":"coordinator","agent":"claude","agent_family":null,"model":null,"effort":null,
            "created_at":"2026-01-02T03:04:05Z","workspace":env.root,"brief":lead.join("lead.md"),
            "lead_pane":"%9","session_link":{"family":"claude","session_id":"lead-session"}
        }))
        .unwrap(),
    )
    .unwrap();
    let store = env.home.join(".claude/projects/project");
    fs::create_dir_all(&store).unwrap();
    for (id, tokens) in [("worker-session", 7), ("lead-session", 11)] {
        atomic_write(store.join(format!("{id}.jsonl")), format!(
            "{{\"type\":\"assistant\",\"timestamp\":\"2026-01-02T04:00:00Z\",\"message\":{{\"id\":\"{id}\",\"model\":\"opus\",\"usage\":{{\"input_tokens\":{tokens},\"output_tokens\":2,\"cache_read_input_tokens\":3,\"cache_creation_input_tokens\":4}}}}}}\n"
        )).unwrap();
    }
    let output = env
        .niles(&env.root, &["usage"])
        .env("TMUX_PANE", "%9")
        .output()
        .unwrap();
    assert_command_success("usage", &output);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = json.get("sessions").unwrap().as_array().unwrap();
    let [_, lead] = array(rows);
    let paths = ["/id", "/skill", "/entry", "/agent", "/usage/input_tokens"];
    assert_eq!(
        serde_json::json!(
            rows.iter()
                .map(|row| paths.map(|path| row.pointer(path).unwrap()))
                .collect::<Vec<_>>()
        ),
        serde_json::json!([
            ["impl", "lead", false, "codex", 7],
            ["lead-one", "coordinator", true, "claude", 11]
        ])
    );
    assert_eq!(lead.get("state"), Some(&serde_json::json!("working")));
    assert!(lead.pointer("/usage/state").is_none());
    for flag in [
        "TMUX_PANE_MISSING",
        "TMUX_PANE_EMPTY",
        "TMUX_PANE_UNCERTAIN",
        "TMUX_PANE_DEAD",
    ] {
        let output = env
            .niles(&env.root, &["usage"])
            .env("TMUX_PANE", "%9")
            .env(flag, "1")
            .output()
            .unwrap();
        assert_command_success("usage with unavailable lead", &output);
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json_field(&json, "/sessions/1/state"), "waiting");
        assert_eq!(json_field(&json, "/sessions/1/usage/input_tokens"), 11);
    }
}
