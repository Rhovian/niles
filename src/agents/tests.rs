use super::*;

fn models() -> ModelRoster {
    ModelRoster::builtin().unwrap()
}

#[test]
fn parses_agent_model_effort_specs() {
    for (input, family, model, effort) in [
        ("codex:gpt-5.5:xhigh", "codex", "gpt-5.5", Some("xhigh")),
        ("claude:sonnet:med", "claude", "sonnet", Some("medium")),
        ("Claude:Opus:MAX", "claude", "opus", Some("max")),
        ("Claude:Haiku", "claude", "haiku", None),
        (
            "CODEX:GPT-5.6-LUNA:MAX",
            "codex",
            "gpt-5.6-luna",
            Some("max"),
        ),
    ] {
        let spec = AgentSpec::parse(input, &models()).unwrap();
        assert_eq!(
            (spec.family(), spec.model(), spec.effort()),
            (family, Some(model), effort),
            "{input}"
        );
        if input == "codex:gpt-5.5:xhigh" {
            assert_eq!(spec.canonical(), input);
        }
    }
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
    - &entry {effort: low, tier: economy, description: colon fixture, speed: null, context_window: 1024}
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
    for (family, model, effort) in [
        ("claude", "claude-opus-5", ""),
        ("codex", "gpt-5.5-codex", ""),
        ("codex", "gpt-5.4", ""),
        ("codex", "omega", ""),
        ("codex", "anthropic/claude-opus-5", ""),
        ("pi", "anthropic/claude-opus-5", ""),
        ("pi", "x-ai/grok-4.6", ""),
        ("pi", "hy3", ""),
        ("codex", "gpt-5.5-codex", ":turbo"),
    ] {
        let spec = AgentSpec::parse(&format!("{family}:{model}{effort}"), &models()).unwrap();
        let error = validate_model(&spec, &models()).unwrap_err().to_string();
        assert!(
            error.contains(&format!("unsupported {family} model `{model}`")),
            "{error}"
        );
    }
}

#[test]
fn rejects_invalid_agent_specs() {
    for spec in [
        "custom",
        "codex:gpt-5.5:xhigh:extra",
        "codex::xhigh",
        "custom:model:high",
        "claude:opus:turbo",
        "codex:gpt-5.5:turbo",
        "codex:not a model:high",
    ] {
        assert!(AgentSpec::parse(spec, &models()).is_err(), "{spec}");
    }
}

#[test]
fn invocations_map_models_efforts_and_manager_defaults() {
    for (spec, defaults, binary, args) in [
        (
            "claude:opus:max",
            InvocationDefaults::Foreground,
            "claude",
            vec!["--model", "opus", "--effort", "max"],
        ),
        (
            "codex:gpt-5.5:xhigh",
            InvocationDefaults::Worker,
            "codex",
            vec![
                "--no-daemon",
                "--dangerously-bypass-approvals-and-sandbox",
                "--model",
                "gpt-5.5",
                "--config",
                "model_reasoning_effort=\"xhigh\"",
            ],
        ),
        (
            "claude:opus:max",
            InvocationDefaults::Worker,
            "claude",
            vec![
                "--dangerously-skip-permissions",
                "--model",
                "opus",
                "--effort",
                "max",
            ],
        ),
    ] {
        let invocation = invocation(spec, defaults, &models()).unwrap();
        assert_eq!(invocation.binary, binary, "{spec}");
        assert_eq!(invocation.args, args, "{spec}");
    }
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
            &roster,
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
