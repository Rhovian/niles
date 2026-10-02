use super::support::*;

#[test]
fn spawn_outside_tmux_fails_with_guidance_instead_of_inventing_a_session() {
    let env = TestEnv::new("niles-worker-no-tmux");

    let spawn = env
        .niles(
            &env.root,
            &["spawn", "auth-fix", "--agent", "claude", "Fix"],
        )
        .env_remove("TMUX")
        .output()
        .unwrap();

    assert_failure_contains("spawn outside tmux", &spawn, "must run inside tmux");
    let stderr = String::from_utf8_lossy(&spawn.stderr);
    assert!(stderr.contains("bare `niles`"), "{stderr}");
    // Refusing is the point: no window, no session, no worker directory left behind.
    assert!(!env.root.join(".niles/worker/auth-fix").exists());
    assert!(!env.tmux_log.exists());
}

#[test]
fn spawn_maps_model_effort_specs_into_worker_launches_and_metadata() {
    let env = TestEnv::new("niles-worker-tier-test");
    fs::write(
        env.root.join("niles.yaml"),
        "models: { codex: { gpt-5.7: { efforts: [xhigh] } } }\n",
    )
    .unwrap();

    let codex_spawn = env
        .niles(
            &env.root,
            &[
                "spawn",
                "codex-hi",
                "--agent",
                "codex:gpt-5.7:xhigh",
                "Fix",
                "auth",
            ],
        )
        .output()
        .unwrap();
    assert_command_success("codex tiered spawn", &codex_spawn);
    let codex_stdout = String::from_utf8_lossy(&codex_spawn.stdout);
    assert!(codex_stdout.contains("agent: codex:gpt-5.7:xhigh"));
    assert!(codex_stdout.contains("agent_family: codex"));
    assert!(codex_stdout.contains("model: gpt-5.7"));
    assert!(codex_stdout.contains("effort: xhigh"));

    let codex_meta = fs::read_to_string(env.root.join(".niles/worker/codex-hi/meta.json")).unwrap();
    let codex_meta_json: serde_json::Value = serde_json::from_str(&codex_meta).unwrap();
    assert!(codex_meta.contains(r#""agent": "codex:gpt-5.7:xhigh""#));
    assert!(codex_meta.contains(r#""role": "worker""#));
    chrono::DateTime::parse_from_rfc3339(codex_meta_json["created_at"].as_str().unwrap()).unwrap();
    assert!(codex_meta.contains(r#""agent_family": "codex""#));
    assert!(codex_meta.contains(r#""model": "gpt-5.7""#));
    assert!(codex_meta.contains(r#""effort": "xhigh""#));

    let codex_launch =
        fs::read_to_string(env.root.join(".niles/worker/codex-hi/launch.sh")).unwrap();
    assert!(codex_launch.contains("'--dangerously-bypass-approvals-and-sandbox'"));
    assert!(codex_launch.contains("'--model' 'gpt-5.7'"));
    assert!(codex_launch.contains("'--config' 'model_reasoning_effort=\"xhigh\"'"));
}

#[test]
fn spawn_rejects_invalid_input_before_writing_a_worker() {
    let workspace = temp_workspace("niles-worker-invalid-input");
    for (args, needle, dir) in [
        (
            &["spawn", "archive", "--agent", "claude", "Fix", "auth"][..],
            "worker id 'archive' is reserved",
            "archive",
        ),
        (
            &[
                "spawn", "auth-fix", "--task", "archive", "--agent", "claude", "Fix", "auth",
            ][..],
            "task label 'archive' is reserved",
            "auth-fix",
        ),
        (
            &["spawn", "bad-worker", "--agent", "claude:opus:turbo", "Fix"][..],
            "unsupported claude effort `turbo`",
            "bad-worker",
        ),
        (
            &[
                "spawn",
                "future-worker",
                "--agent",
                "codex:gpt-5.7:xhigh",
                "Fix",
            ][..],
            "unsupported codex model `gpt-5.7`",
            "future-worker",
        ),
    ] {
        let output = niles_bare(&workspace, &niles_home(&workspace))
            .args(args)
            .output()
            .unwrap();
        assert_failure_contains("invalid spawn", &output, needle);
        assert!(!workspace.join(".niles/worker").join(dir).exists());
    }
}
