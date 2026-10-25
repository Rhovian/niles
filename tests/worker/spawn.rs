use super::support::*;

#[test]
fn manifest_tiers_bound_spawn_before_worker_state() {
    let env = TestEnv::with_manifest("niles-worker-groups", "claude");
    let path = state_dir(&env.root).join("manifest.yaml");
    let body = fs::read_to_string(&path).unwrap().replace(
        "research:\n  standard: {primary: claude:sonnet:high, secondaries: []}",
        "research:\n  standard: {primary: codex:gpt-6-sol:medium, secondaries: [claude:haiku]}\n  economy: {primary: claude:haiku, secondaries: []}",
    );
    fs::write(&path, body).unwrap();
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
    }
    assert_failure_contains(
        "unconfigured tier",
        &env.spawn("design", &["tier", "--tier", "economy", "Task"]),
        "does not list tier economy",
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
    assert_eq!(listing["workers"][0]["tree"], tree_arg);
}

#[test]
fn spawn_rejects_missing_tree_without_worker_state() {
    let env = TestEnv::with_manifest("niles-worker-missing-tree", "claude");
    let missing = env.root.join("missing");
    let spawn = env.spawn(
        "research",
        &["other", "--tree", missing.to_str().unwrap(), "Fix"],
    );
    assert_failure_contains("missing tree", &spawn, "failed to resolve tree");
    assert!(!state_dir(&env.root).join("worker/other").exists());
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
    let stderr = String::from_utf8_lossy(&spawn.stderr);
    assert!(stderr.contains("bare `niles`"), "{stderr}");
    // Refusing is the point: no window, no session, no worker directory left behind.
    assert!(!state_dir(&env.root).join("worker/auth-fix").exists());
    assert!(!env.tmux_log.exists());
}

#[test]
fn spawn_maps_model_effort_specs_into_worker_launches_and_metadata() {
    let env = TestEnv::with_manifest("niles-worker-tier-test", "codex:gpt-5.7:xhigh");
    fs::write(
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
    let codex_stdout = String::from_utf8_lossy(&codex_spawn.stdout);
    assert!(codex_stdout.contains("agent: codex:gpt-5.7:xhigh"));
    assert!(codex_stdout.contains("agent_family: codex"));
    assert!(codex_stdout.contains("model: gpt-5.7"));
    assert!(codex_stdout.contains("effort: xhigh"));

    let codex_meta =
        fs::read_to_string(state_dir(&env.root).join("worker/codex-hi/meta.json")).unwrap();
    let codex_meta_json: serde_json::Value = serde_json::from_str(&codex_meta).unwrap();
    assert!(codex_meta.contains(r#""agent": "codex:gpt-5.7:xhigh""#));
    assert!(codex_meta.contains(r#""skill": "research""#));
    chrono::DateTime::parse_from_rfc3339(codex_meta_json["created_at"].as_str().unwrap()).unwrap();
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
    let workspace = temp_workspace("niles-worker-invalid-input");
    write_spawn_manifest(&workspace, "claude");
    for (id, flags, error) in [
        ("archive", &[][..], "worker id 'archive' is reserved"),
        (
            "auth-fix",
            &["--task", "archive"],
            "task label 'archive' is reserved",
        ),
        (
            "bad-worker",
            &["--agent", "claude:opus:turbo"],
            "unsupported claude effort `turbo`",
        ),
        (
            "future-worker",
            &["--agent", "codex:gpt-5.7:xhigh"],
            "unsupported codex model `gpt-5.7`",
        ),
    ] {
        let output = niles_bare(&workspace, &niles_home(&workspace))
            .args(["spawn", id, "--skill", "research"])
            .args(flags)
            .arg("Task")
            .output()
            .unwrap();
        assert_failure_contains("invalid spawn", &output, error);
        assert!(!state_dir(&workspace).join("worker").join(id).exists());
    }
}

#[test]
fn entry_and_unbound_skills_fail_before_worker_state_even_with_override() {
    let env = TestEnv::with_manifest("spawn-bound-skill", "claude");
    let manifest = state_dir(&env.root).join("manifest.yaml");
    let body = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        format!("{body}missing: {{standard: {{primary: claude:sonnet:high, secondaries: []}}}}\n"),
    )
    .unwrap();
    for (skill, error) in [
        ("lead", "entry skill"),
        (
            "unbound",
            "bound skills: design, lead, missing, research, reviewer, security, worker",
        ),
        ("missing", "no compiled primary skill missing"),
    ] {
        assert_failure_contains(
            skill,
            &env.spawn(skill, &["job", "--agent", "claude:sonnet:high", "task"]),
            error,
        );
        assert!(!state_dir(&env.root).join("worker/job").exists());
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
