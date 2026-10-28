use super::*;

fn models() -> ModelRoster {
    ModelRoster::builtin().unwrap()
}

#[test]
fn foreground_invocation_preserves_builtin_manager_defaults() {
    let invocation =
        invocation("claude:opus:max", InvocationDefaults::Foreground, &models()).unwrap();

    assert_eq!(invocation.binary, "claude");
    assert_eq!(
        invocation.args,
        ["--model", "opus", "--effort", "max"].map(str::to_owned)
    );
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

    assert!(AgentSpec::parse("custom", &models()).is_err());
}

#[test]
fn only_supported_families_are_launchable() {
    assert_eq!(
        known_agent_ids().collect::<Vec<_>>(),
        ["codex", "claude", "pi"]
    );
    for spec in ["hermes", "hermes:z-ai/glm-5.3-flash:high"] {
        let error = invocation(spec, InvocationDefaults::Worker, &models()).unwrap_err();
        assert!(error.to_string().contains("unknown agent family `hermes`"));
    }
}

#[test]
fn model_names_may_contain_colons() {
    let models = roster::parse(
        r#"pi:
  nvidia/nemotron:free:
    - &entry {effort: low, tier: economy, description: colon fixture, cost: null, speed: null, context_window: 1024}
    - {<<: *entry, effort: high, tier: standard}
"#,
    )
    .unwrap();
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
        ("pi", "anthropic/claude-opus-5"),
        ("pi", "x-ai/grok-4.6"),
        ("pi", "hy3"),
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
    let invocation =
        invocation("codex:gpt-5.5:xhigh", InvocationDefaults::Worker, &models()).unwrap();

    assert_eq!(invocation.binary, "codex");
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
    let invocation = invocation("claude:opus:max", InvocationDefaults::Worker, &models()).unwrap();

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
fn headless_invocation_orders_worker_headless_and_tier_args() {
    for family in known_agent_ids() {
        let profile = profile_for(family).unwrap();
        let roster = models();
        let model = roster.model_names(family).next().unwrap();
        let invocation = invocation(
            &format!("{family}:{model}:low"),
            InvocationDefaults::Headless,
            &models(),
        )
        .unwrap();
        let expected: Vec<String> = profile
            .worker_args
            .iter()
            .chain(profile.headless)
            .map(|arg| (*arg).to_owned())
            .chain(families::tier_args(profile, Some(model), Some("low")))
            .collect();
        assert_eq!(invocation.args, expected, "{family}");
        if family == "codex" {
            assert_eq!(
                invocation.args,
                [
                    "--no-daemon",
                    "--dangerously-bypass-approvals-and-sandbox",
                    "exec",
                    "--skip-git-repo-check",
                    "--model",
                    "gpt-5.5",
                    "--config",
                    "model_reasoning_effort=\"low\"",
                ]
            );
        }
    }
}
