use super::*;
use crate::config::spec::PromptMode;

fn models() -> ModelRoster {
    ModelRoster::builtin().unwrap()
}

#[test]
fn foreground_invocation_preserves_builtin_manager_defaults() {
    let invocation = invocation(
        "claude:opus:max",
        None,
        InvocationDefaults::Foreground,
        &models(),
    )
    .unwrap();

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

    let invocation = invocation(
        "gemini",
        Some(&config),
        InvocationDefaults::Foreground,
        &models(),
    )
    .unwrap();

    assert_eq!(invocation.binary, "/tmp/custom-manager");
    assert_eq!(invocation.args, ["--mode", "manager"].map(str::to_owned));
    assert_eq!(invocation.spec.family(), "gemini");
    assert!(matches!(invocation.brief, BriefDelivery::Arg));
}

#[test]
fn parses_agent_model_effort_specs() {
    let codex = AgentSpec::parse("codex:gpt-5.5:xhigh", &models()).unwrap();
    assert_eq!(codex.canonical(), "codex:gpt-5.5:xhigh");
    assert_eq!(codex.family(), "codex");
    assert_eq!(codex.model(), Some("gpt-5.5"));
    assert_eq!(codex.effort(), Some("xhigh"));

    let claude = AgentSpec::parse("claude:sonnet:med", &models()).unwrap();
    assert_eq!(claude.family(), "claude");
    assert_eq!(claude.model(), Some("sonnet"));
    assert_eq!(claude.effort(), Some("medium"));

    let opus = AgentSpec::parse("Claude:Opus:MAX", &models()).unwrap();
    assert_eq!(opus.family(), "claude");
    assert_eq!(opus.model(), Some("opus"));
    assert_eq!(opus.effort(), Some("max"));

    let haiku = AgentSpec::parse("Claude:Haiku", &models()).unwrap();
    assert_eq!(haiku.model(), Some("haiku"));
    assert_eq!(haiku.effort(), None);

    let uppercase = AgentSpec::parse("CODEX:GPT-5.6-LUNA:MAX", &models()).unwrap();
    assert_eq!(uppercase.family(), "codex");
    assert_eq!(uppercase.model(), Some("gpt-5.6-luna"));
    assert_eq!(uppercase.effort(), Some("max"));

    let bare = AgentSpec::parse("custom", &models()).unwrap();
    assert_eq!(bare.family(), "custom");
    assert_eq!(bare.model(), None);
    assert_eq!(bare.effort(), None);
}

#[test]
fn model_names_may_contain_colons() {
    let models = roster::parse("pi:\n  nvidia/nemotron:free:\n    efforts: [low, high]\n").unwrap();
    let bare = AgentSpec::parse("pi:nvidia/nemotron:free", &models).unwrap();
    assert_eq!(
        (bare.model(), bare.effort()),
        (Some("nvidia/nemotron:free"), None)
    );
    let tiered = AgentSpec::parse("pi:nvidia/nemotron:free:high", &models).unwrap();
    assert_eq!(tiered.canonical(), "pi:nvidia/nemotron:free:high");
    assert!(AgentSpec::parse("codex::high", &models).is_err());
}

#[test]
fn effort_is_checked_against_the_model_not_the_family() {
    // The ladders the codex CLI reports: astra climbs to `ultra`, luna stops at `max`, and 5.5
    // stops at `xhigh`. One family-wide list could not tell these apart.
    AgentSpec::parse("codex:gpt-6-astra:ultra", &models()).unwrap();
    AgentSpec::parse("codex:gpt-5.6-luna:max", &models()).unwrap();
    AgentSpec::parse("codex:gpt-5.5:xhigh", &models()).unwrap();

    for spec in ["codex:gpt-5.6-luna:ultra", "codex:gpt-5.5:max"] {
        let error = AgentSpec::parse(spec, &models()).unwrap_err().to_string();
        assert!(error.contains("unsupported codex effort"), "{error}");
        assert!(error.contains("for model"), "{error}");
    }

    let error = AgentSpec::parse("claude:haiku:low", &models())
        .unwrap_err()
        .to_string();
    assert!(error.contains("takes no effort"), "{error}");
}

#[test]
fn a_model_off_the_roster_is_rejected_whatever_it_looks_like() {
    // A full id, a plausible sibling, a future slug: the roster is the whole rule, so none of them
    // are launchable until they are a line in it.
    for (family, model) in [
        ("claude", "claude-opus-5"),
        ("codex", "gpt-5.5-codex"),
        ("codex", "gpt-5.4"),
        ("codex", "omega"),
        ("codex", "anthropic/claude-opus-5"),
        ("hermes", "anthropic/claude-opus-5"),
        ("hermes", "x-ai/grok-4.6"),
        ("hermes", "hy3"),
    ] {
        let spec = AgentSpec::parse(&format!("{family}:{model}"), &models()).unwrap();
        let error = validate_model(&spec, &models()).unwrap_err().to_string();
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
    let spec = AgentSpec::parse("codex:gpt-5.5-codex:turbo", &models()).unwrap();
    let error = validate_model(&spec, &models()).unwrap_err().to_string();

    assert!(
        error.contains("unsupported codex model `gpt-5.5-codex`"),
        "{error}"
    );
}

#[test]
fn rejects_invalid_agent_specs() {
    assert!(AgentSpec::parse("codex:gpt-5.5:xhigh:extra", &models()).is_err());
    assert!(AgentSpec::parse("codex::xhigh", &models()).is_err());
    assert!(AgentSpec::parse("custom:model:high", &models()).is_err());
    assert!(AgentSpec::parse("claude:opus:turbo", &models()).is_err());
    assert!(AgentSpec::parse("codex:gpt-5.5:turbo", &models()).is_err());
    assert!(AgentSpec::parse("codex:not a model:high", &models()).is_err());
}

#[test]
fn invocation_maps_codex_model_effort_flags() {
    let invocation = invocation(
        "codex:gpt-5.5:xhigh",
        None,
        InvocationDefaults::Worker,
        &models(),
    )
    .unwrap();

    assert_eq!(invocation.binary, "codex");
    assert!(matches!(invocation.brief, BriefDelivery::Arg));
    assert_eq!(
        invocation.args,
        [
            "--no-daemon",
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
    let invocation = invocation(
        "claude:opus:max",
        None,
        InvocationDefaults::Worker,
        &models(),
    )
    .unwrap();

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
    let invocation = invocation(
        "hermes:tencent/hy3:high",
        None,
        InvocationDefaults::Worker,
        &models(),
    )
    .unwrap();

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
    let invocation = invocation("hermes", None, InvocationDefaults::Foreground, &models()).unwrap();

    assert_eq!(invocation.binary, "hermes");
    assert_eq!(invocation.args, ["chat"].map(str::to_owned));
}

#[test]
fn hermes_carries_a_roster_like_every_other_family() {
    // hermes routes to any provider its config knows, but Niles launches only what it carries:
    // another vendor path is a line in the roster, not a shape we wave through.
    for model in [
        "tencent/hy3",
        "deepseek/deepseek-v4.1-flash",
        "z-ai/glm-5.3-flash",
    ] {
        let spec = AgentSpec::parse(&format!("hermes:{model}"), &models()).unwrap();
        validate_model(&spec, &models()).unwrap();
    }
}

#[test]
fn hermes_reasoning_levels_cover_the_cli_vocabulary() {
    for effort in [
        "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
    ] {
        AgentSpec::parse(&format!("hermes:tencent/hy3:{effort}"), &models()).unwrap();
    }

    let unsupported = AgentSpec::parse("hermes:tencent/hy3:turbo", &models()).unwrap_err();
    assert!(
        unsupported
            .to_string()
            .contains("unsupported hermes effort"),
        "{unsupported}"
    );
}

#[test]
fn hermes_sources_are_unique_per_launch() {
    let first = super::session_link("hermes", "worker", camino::Utf8Path::new("/state")).unwrap();
    let second = super::session_link("hermes", "worker", camino::Utf8Path::new("/state")).unwrap();
    let (
        crate::telemetry::SessionLink::Hermes { source: a },
        crate::telemetry::SessionLink::Hermes { source: b },
    ) = (first, second)
    else {
        panic!("hermes must have a source")
    };
    assert!(a.starts_with("niles:worker:"));
    assert_ne!(a, b);
}
