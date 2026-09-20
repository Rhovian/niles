use super::*;

#[test]
fn known_and_unknown_agents_resolve_a_binary_name() {
    assert_eq!(default_binary("codex"), "codex");
    assert_eq!(default_binary("custom"), "custom");
}

#[test]
fn invocation_applies_worker_defaults() {
    let invocation = invocation("codex", None, InvocationDefaults::Worker).unwrap();

    assert_eq!(invocation.binary, "codex");
    assert_eq!(
        invocation.args,
        ["--dangerously-bypass-approvals-and-sandbox"].map(str::to_owned)
    );
    assert!(matches!(invocation.prompt, PromptMode::Arg));
}

#[test]
fn foreground_invocation_preserves_builtin_manager_defaults() {
    let invocation = foreground_invocation("claude:opus:max", None).unwrap();

    assert_eq!(invocation.binary, "claude");
    assert_eq!(
        invocation.args,
        ["--model", "opus", "--effort", "max"].map(str::to_owned)
    );
    assert!(matches!(invocation.prompt, PromptMode::Arg));
}

#[test]
fn foreground_invocation_uses_configured_custom_manager_binary_and_args() {
    let config = AgentConfig {
        binary: Some("/tmp/custom-manager".to_owned()),
        args: ["--mode", "manager"].map(str::to_owned).to_vec(),
        prompt: PromptMode::Arg,
    };

    let invocation = foreground_invocation("gemini", Some(&config)).unwrap();

    assert_eq!(invocation.binary, "/tmp/custom-manager");
    assert_eq!(invocation.args, ["--mode", "manager"].map(str::to_owned));
    assert_eq!(invocation.spec.family(), "gemini");
    assert!(matches!(invocation.prompt, PromptMode::Arg));
}

#[test]
fn parses_agent_model_effort_specs() {
    let codex = AgentSpec::parse("codex:gpt-5.5:xhigh").unwrap();
    assert_eq!(codex.canonical(), "codex:gpt-5.5:xhigh");
    assert_eq!(codex.family(), "codex");
    assert_eq!(codex.model(), Some("gpt-5.5"));
    assert_eq!(codex.effort(), Some("xhigh"));

    let claude = AgentSpec::parse("claude:sonnet:med").unwrap();
    assert_eq!(claude.family(), "claude");
    assert_eq!(claude.model(), Some("sonnet"));
    assert_eq!(claude.effort(), Some("medium"));

    let haiku = AgentSpec::parse("Claude:Haiku:MAX").unwrap();
    assert_eq!(haiku.family(), "claude");
    assert_eq!(haiku.model(), Some("haiku"));
    assert_eq!(haiku.effort(), Some("max"));

    let full_claude = AgentSpec::parse("claude:claude-haiku-4-5-20251001:low").unwrap();
    assert_eq!(full_claude.model(), Some("claude-haiku-4-5-20251001"));

    let codex_full = AgentSpec::parse("CODEX:gpt-5.5-codex:xhigh").unwrap();
    assert_eq!(codex_full.family(), "codex");
    assert_eq!(codex_full.model(), Some("gpt-5.5-codex"));

    let future = AgentSpec::parse("codex:omega:xhigh").unwrap();
    assert_eq!(future.model(), Some("omega"));

    let bare = AgentSpec::parse("custom").unwrap();
    assert_eq!(bare.family(), "custom");
    assert_eq!(bare.model(), None);
    assert_eq!(bare.effort(), None);
}

#[test]
fn codex_static_aliases_prefer_gpt_5_5() {
    assert_eq!(default_model_aliases("codex")[0], "gpt-5.5");
}

#[test]
fn rejects_invalid_agent_specs() {
    assert!(AgentSpec::parse("codex:gpt-5.5:xhigh:extra").is_err());
    assert!(AgentSpec::parse("codex::xhigh").is_err());
    assert!(AgentSpec::parse("custom:model:high").is_err());
    assert!(AgentSpec::parse("claude:opus:turbo").is_err());
    assert!(AgentSpec::parse("codex:gpt-5.5:turbo").is_err());
    assert!(AgentSpec::parse("codex:not a model:high").is_err());
}

#[test]
fn static_validation_rejects_unknown_builtin_models() {
    let spec = AgentSpec::parse("codex:omega:high").unwrap();
    let err = validate_static_model(&spec).unwrap_err().to_string();

    assert!(err.contains("unsupported codex model `omega`"));
}

#[test]
fn invocation_maps_codex_model_effort_flags() {
    let invocation = invocation("codex:gpt-5.5:xhigh", None, InvocationDefaults::Worker).unwrap();

    assert_eq!(invocation.binary, "codex");
    assert_eq!(
        invocation.args,
        [
            "--dangerously-bypass-approvals-and-sandbox",
            "--model",
            "gpt-5.5",
            "--config",
            "model_reasoning_effort=\"xhigh\""
        ]
        .map(str::to_owned)
    );
}

#[test]
fn invocation_maps_claude_model_effort_flags() {
    let invocation = invocation("claude:opus:max", None, InvocationDefaults::Worker).unwrap();

    assert_eq!(invocation.binary, "claude");
    assert_eq!(
        invocation.args,
        [
            "--dangerously-skip-permissions",
            "--model",
            "opus",
            "--effort",
            "max"
        ]
        .map(str::to_owned)
    );
}

#[test]
fn hermes_worker_runs_the_chat_subcommand_and_reads_the_brief_from_a_file() {
    let invocation =
        invocation("hermes:tencent/hy3:high", None, InvocationDefaults::Worker).unwrap();

    assert_eq!(invocation.binary, "hermes");
    assert_eq!(
        invocation.args,
        [
            "chat",
            "--yolo",
            "--model",
            "tencent/hy3",
            "--reasoning",
            "high"
        ]
        .map(str::to_owned)
    );
    assert!(matches!(invocation.prompt, PromptMode::QueryFile));
}

#[test]
fn hermes_foreground_keeps_the_subcommand_without_the_approval_bypass() {
    let invocation = foreground_invocation("hermes", None).unwrap();

    assert_eq!(invocation.binary, "hermes");
    assert_eq!(invocation.args, ["chat"].map(str::to_owned));
}

#[test]
fn hermes_takes_any_vendor_path_model_but_not_a_bare_name() {
    for model in ["tencent/hy3", "anthropic/claude-opus-5", "x-ai/grok-4.6"] {
        validate_static_model(&AgentSpec::parse(&format!("hermes:{model}")).unwrap()).unwrap();
    }

    let bare = AgentSpec::parse("hermes:hy3").unwrap();
    let error = validate_static_model(&bare).unwrap_err().to_string();
    assert!(error.contains("unsupported hermes model"), "{error}");
}

#[test]
fn a_vendor_slash_stays_out_of_the_prefixed_families() {
    let claude = AgentSpec::parse("claude:anthropic/claude-opus-5").unwrap();
    let error = validate_static_model(&claude).unwrap_err().to_string();
    assert!(error.contains("unsupported claude model"), "{error}");
}

#[test]
fn hermes_reasoning_levels_cover_the_cli_vocabulary() {
    for effort in [
        "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
    ] {
        AgentSpec::parse(&format!("hermes:tencent/hy3:{effort}")).unwrap();
    }

    let unsupported = AgentSpec::parse("hermes:tencent/hy3:turbo").unwrap_err();
    assert!(
        unsupported
            .to_string()
            .contains("unsupported hermes effort"),
        "{unsupported}"
    );
}
