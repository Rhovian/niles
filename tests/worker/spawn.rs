use super::support::*;

#[test]
fn manifest_tiers_bound_spawn_before_worker_state() {
    let env = TestEnv::with_manifest("niles-worker-groups", "claude");
    let path = state_dir(&env.root).join("manifest.yaml");
    let body = "entry: lead\nentry_agent: codex:gpt-6-sol:medium\ntiers:\n  standard: {primary: codex:gpt-6-sol:medium, secondaries: [claude:haiku]}\n  economy: {primary: claude:haiku, secondaries: []}\n";
    atomic_write(&path, body).unwrap();
    for (id, flags, agent) in [
        ("default", &[][..], "codex:gpt-6-sol:medium"),
        ("economy", &["--tier", "economy"], "claude:haiku"),
        ("listed", &["--agent", "claude:haiku"], "claude:haiku"),
    ] {
        let result = env
            .spawn_command("research", &[id])
            .args(flags)
            .arg("Task")
            .output()
            .unwrap();
        assert_command_success(id, &result);
        let meta = fs::read_to_string(state_dir(&env.root).join(format!("worker/{id}/meta.json")))
            .unwrap();
        assert!(meta.contains(&format!("\"agent\": \"{agent}\"")), "{meta}");
        assert!(!meta.contains("\"setup\""));
    }
    let launch = env.spawn(
        "design",
        &[
            "any-skill",
            "--tier",
            "economy",
            "--menu",
            "reviewer",
            "--with",
            "security",
            "Task",
        ],
    );
    assert_command_success("unbound skill with shared tier", &launch);
    assert_failure_contains(
        "unconfigured tier",
        &env.spawn("design", &["tier", "--tier", "frontier", "Task"]),
        "does not list tier frontier",
    );
    for (id, agent) in [
        ("unlisted", "codex:gpt-6-luna:medium"),
        ("effort", "codex:gpt-6-sol:low"),
        ("missing", "codex:gpt-6-sol"),
    ] {
        let result = env.spawn("research", &[id, "--agent", agent, "Task"]);
        assert_failure_contains(id, &result, "allowed agents");
        assert!(!state_dir(&env.root).join("worker").join(id).exists());
    }
}

#[test]
fn spawn_in_another_tree_keeps_worker_state_in_workspace() {
    let env = TestEnv::with_manifest("niles-worker-tree", "claude");
    let tree = env.root.join("other-tree");
    fs::create_dir(&tree).unwrap();
    let tree_arg = tree.to_str().unwrap();
    let spawn = env.spawn("research", &["other", "--tree", tree_arg, "Fix"]);
    assert_command_success("spawn in another tree", &spawn);
    assert!(env.tmux_log().contains(&format!("-c {tree_arg} ")));
    let workers = env.run(&["workers"]);
    assert_command_success("workers with tree", &workers);
    let listing: serde_json::Value = serde_json::from_slice(&workers.stdout).unwrap();
    assert_eq!(json_field(&listing, "/workers/0/tree"), tree_arg);
}

#[test]
fn spawn_outside_tmux_fails_with_guidance_instead_of_inventing_a_session() {
    let env = TestEnv::with_manifest("niles-worker-no-tmux", "claude");

    let spawn = env
        .spawn_command("research", &["auth-fix", "Fix"])
        .env_remove("TMUX")
        .output()
        .unwrap();

    assert_failure_contains("spawn outside tmux", &spawn, "must run inside tmux");
    let stderr = stderr_of(&spawn);
    assert!(stderr.contains("bare `niles`"), "{stderr}");
    // Refusing is the point: no window, no session, no worker directory left behind.
    assert!(!state_dir(&env.root).join("worker/auth-fix").exists());
    assert!(!env.tmux_log.exists());
}

#[test]
fn spawn_maps_model_effort_specs_into_worker_launches_and_metadata() {
    let env = TestEnv::with_manifest("niles-worker-tier-test", "codex:gpt-5.7:xhigh");
    atomic_write(
        env.root.join("niles.yaml"),
        "models: { codex: { gpt-5.7: { efforts: [xhigh] } } }\n",
    )
    .unwrap();

    let codex_spawn = env
        .spawn_command(
            "research",
            &["codex-hi", "--agent", "codex:gpt-5.7:xhigh", "Fix", "auth"],
        )
        .output()
        .unwrap();
    assert_command_success("codex tiered spawn", &codex_spawn);
    let codex_stdout = stdout_of(&codex_spawn);
    assert!(codex_stdout.contains("agent: codex:gpt-5.7:xhigh"));
    assert!(codex_stdout.contains("agent_family: codex"));
    assert!(codex_stdout.contains("model: gpt-5.7"));
    assert!(codex_stdout.contains("effort: xhigh"));

    let codex_meta =
        fs::read_to_string(state_dir(&env.root).join("worker/codex-hi/meta.json")).unwrap();
    let codex_meta_json: serde_json::Value = serde_json::from_str(&codex_meta).unwrap();
    assert!(codex_meta.contains(r#""agent": "codex:gpt-5.7:xhigh""#));
    assert!(codex_meta.contains(r#""skill": "research""#));
    chrono::DateTime::parse_from_rfc3339(
        codex_meta_json
            .pointer("/created_at")
            .unwrap()
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert!(codex_meta.contains(r#""agent_family": "codex""#));
    assert!(codex_meta.contains(r#""model": "gpt-5.7""#));
    assert!(codex_meta.contains(r#""effort": "xhigh""#));

    let codex_launch =
        fs::read_to_string(state_dir(&env.root).join("worker/codex-hi/launch.sh")).unwrap();
    assert!(codex_launch.contains("'--dangerously-bypass-approvals-and-sandbox'"));
    assert!(codex_launch.contains("'--model' 'gpt-5.7'"));
    assert!(codex_launch.contains("'--config' 'model_reasoning_effort=\"xhigh\"'"));
}

#[test]
fn spawn_rejects_invalid_input_before_writing_a_worker() {
    let env = TestEnv::with_manifest("niles-worker-invalid-input", "claude");
    let missing = env.root.join("missing");
    for (id, skill, flags, error, bare) in [
        (
            "archive",
            "research",
            &[][..],
            "worker id 'archive' is reserved",
            true,
        ),
        (
            "auth-fix",
            "research",
            &["--task", "archive"],
            "task label 'archive' is reserved",
            true,
        ),
        (
            "bad-worker",
            "research",
            &["--agent", "claude:opus:turbo"],
            "invalid claude effort `turbo`",
            true,
        ),
        (
            "future-worker",
            "research",
            &["--agent", "codex:gpt-5.7:xhigh"],
            "invalid codex model `gpt-5.7`",
            true,
        ),
        (
            "job",
            "lead",
            &["--agent", "claude:sonnet:high"],
            "entry skill",
            false,
        ),
        (
            "job",
            "unbound",
            &["--agent", "claude:sonnet:high"],
            "skill `unbound` has no SKILL.md or SKILL.ref",
            false,
        ),
        (
            "other",
            "research",
            &["--tree", missing.to_str().unwrap()],
            "failed to resolve tree",
            false,
        ),
    ] {
        let mut command = env.spawn_command(skill, &[id]);
        command.args(flags).arg("Task");
        if bare {
            command.env_remove("TMUX");
        }
        let output = command.output().unwrap();
        assert_failure_contains(id, &output, error);
        assert!(!state_dir(&env.root).join("worker").join(id).exists());
    }
}

#[test]
fn a_manifest_is_required_even_with_an_agent_override() {
    let env = TestEnv::new("spawn-missing-manifest");
    for args in [&["job", "task"][..], &["job", "--agent", "claude", "task"]] {
        assert_failure_contains(
            "missing manifest",
            &env.spawn("worker", args),
            "does not exist",
        );
    }
    assert!(!state_dir(&env.root).join("worker/job").exists());
}
