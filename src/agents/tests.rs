use super::*;
use crate::config::spec::PromptMode;

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
    assert!(matches!(invocation.brief, BriefDelivery::Arg));
}

#[test]
fn foreground_invocation_preserves_builtin_manager_defaults() {
    let invocation = foreground_invocation("claude:opus:max", None).unwrap();

    assert_eq!(invocation.binary, "claude");
    assert_eq!(
        invocation.args,
        ["--model", "opus", "--effort", "max"].map(str::to_owned)
    );
    // The lead's dial, not the worker's: claude's brief is standing context for the session it
    // drives, where a worker's brief is the turn.
    assert!(matches!(
        invocation.brief,
        BriefDelivery::SystemPrompt("--append-system-prompt")
    ));
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
    assert!(matches!(invocation.brief, BriefDelivery::Arg));
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

    let opus = AgentSpec::parse("Claude:Opus:MAX").unwrap();
    assert_eq!(opus.family(), "claude");
    assert_eq!(opus.model(), Some("opus"));
    assert_eq!(opus.effort(), Some("max"));

    let haiku = AgentSpec::parse("Claude:Haiku").unwrap();
    assert_eq!(haiku.model(), Some("haiku"));
    assert_eq!(haiku.effort(), None);

    let uppercase = AgentSpec::parse("CODEX:GPT-5.6-LUNA:MAX").unwrap();
    assert_eq!(uppercase.family(), "codex");
    assert_eq!(uppercase.model(), Some("gpt-5.6-luna"));
    assert_eq!(uppercase.effort(), Some("max"));

    let bare = AgentSpec::parse("custom").unwrap();
    assert_eq!(bare.family(), "custom");
    assert_eq!(bare.model(), None);
    assert_eq!(bare.effort(), None);
}

#[test]
fn codex_static_aliases_prefer_gpt_5_5() {
    assert_eq!(model_names("codex")[0], "gpt-5.5");
}

#[test]
fn effort_is_checked_against_the_model_not_the_family() {
    // The ladders the codex CLI reports: astra climbs to `ultra`, luna stops at `max`, and 5.5
    // stops at `xhigh`. One family-wide list could not tell these apart.
    AgentSpec::parse("codex:gpt-6-astra:ultra").unwrap();
    AgentSpec::parse("codex:gpt-5.6-luna:max").unwrap();
    AgentSpec::parse("codex:gpt-5.5:xhigh").unwrap();

    for spec in ["codex:gpt-5.6-luna:ultra", "codex:gpt-5.5:max"] {
        let error = AgentSpec::parse(spec).unwrap_err().to_string();
        assert!(error.contains("unsupported codex effort"), "{error}");
        assert!(error.contains("for model"), "{error}");
    }
}

#[test]
fn a_model_with_no_effort_takes_none() {
    // claude gates effort on a model capability, and the haiku alias resolves to a model that has
    // none of them: there is no level to pass, so the spec says so rather than sending one.
    AgentSpec::parse("claude:haiku").unwrap();

    let error = AgentSpec::parse("claude:haiku:low")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("claude model `haiku` takes no effort"),
        "{error}"
    );

    assert!(supported_efforts("claude", Some("haiku")).is_empty());
    assert_eq!(
        supported_efforts("claude", Some("opus")),
        ["low", "medium", "high", "xhigh", "max"]
    );
}

#[test]
fn a_model_off_the_roster_is_rejected_whatever_it_looks_like() {
    // A full id, a plausible sibling, a future slug: the roster is the whole rule, so none of them
    // are launchable until they are a line in it.
    for model in [
        "claude-opus-5",
        "gpt-5.5-codex",
        "gpt-5.4",
        "omega",
        "anthropic/claude-opus-5",
    ] {
        let family = match model.starts_with("claude") {
            true => "claude",
            false => "codex",
        };
        let spec = AgentSpec::parse(&format!("{family}:{model}")).unwrap();
        let error = validate_static_model(&spec).unwrap_err().to_string();
        assert!(
            error.contains(&format!("unsupported {family} model")),
            "{error}"
        );
    }
}

#[test]
fn an_effort_on_a_model_off_the_roster_reports_the_model() {
    // The model is what is wrong with the spec; saying `turbo` is unsupported would name the half
    // that is beside the point.
    let spec = AgentSpec::parse("codex:gpt-5.5-codex:turbo").unwrap();
    let error = validate_static_model(&spec).unwrap_err().to_string();

    assert!(
        error.contains("unsupported codex model `gpt-5.5-codex`"),
        "{error}"
    );
}

#[test]
fn model_efforts_come_from_the_model_when_one_is_named() {
    assert_eq!(
        supported_efforts("codex", Some("gpt-5.5")),
        ["low", "medium", "high", "xhigh"]
    );
    assert_eq!(
        supported_efforts("codex", Some("gpt-6-astra")),
        ["low", "medium", "high", "xhigh", "max", "ultra"]
    );
    assert!(supported_efforts("codex", Some("gpt-5.5-codex")).is_empty());
    assert!(supported_efforts("custom", Some("anything")).is_empty());
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
    assert!(matches!(
        invocation.brief,
        BriefDelivery::Flag {
            path: "--query-file",
            ..
        }
    ));
}

#[test]
fn hermes_foreground_keeps_the_subcommand_without_the_approval_bypass() {
    let invocation = foreground_invocation("hermes", None).unwrap();

    assert_eq!(invocation.binary, "hermes");
    assert_eq!(invocation.args, ["chat"].map(str::to_owned));
}

#[test]
fn hermes_carries_a_roster_like_every_other_family() {
    // hermes routes to any provider its config knows, but Niles launches only what it carries:
    // another vendor path is a line in the roster, not a shape we wave through.
    validate_static_model(&AgentSpec::parse("hermes:tencent/hy3").unwrap()).unwrap();

    for model in ["anthropic/claude-opus-5", "x-ai/grok-4.6", "hy3"] {
        let spec = AgentSpec::parse(&format!("hermes:{model}")).unwrap();
        let error = validate_static_model(&spec).unwrap_err().to_string();
        assert!(error.contains("unsupported hermes model"), "{error}");
    }
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
