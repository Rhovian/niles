use super::*;

fn models() -> ModelRoster {
    ModelRoster::builtin().unwrap()
}

#[test]
fn parses_agent_model_effort_specs() {
    let max = Some("max");
    for (input, family, model, effort) in [
        ("codex:gpt-5.5:xhigh", "codex", "gpt-5.5", Some("xhigh")),
        ("claude:sonnet:med", "claude", "sonnet", Some("medium")),
        ("Claude:Opus:MAX", "claude", "opus", max),
        ("Claude:Haiku", "claude", "haiku", None),
        ("CODEX:GPT-5.6-LUNA:MAX", "codex", "gpt-5.6-luna", max),
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
        assert!(error.contains("invalid codex effort"), "{error}");
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
            error.contains(&format!("invalid {family} model `{model}`")),
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
fn invocations_map_models_efforts_and_order_headless_defaults() {
    use InvocationDefaults::{Foreground, Headless, Worker};
    for (spec, defaults, args) in [
        ("claude", Foreground, ""),
        ("claude:opus:max", Foreground, "--model opus --effort max"),
        (
            "codex:gpt-5.5:xhigh",
            Worker,
            r#"--no-daemon --dangerously-bypass-approvals-and-sandbox --model gpt-5.5 --config model_reasoning_effort="xhigh""#,
        ),
        (
            "claude:opus:max",
            Worker,
            "--dangerously-skip-permissions --model opus --effort max",
        ),
        (
            "codex:gpt-5.5:low",
            Headless,
            r#"--no-daemon --dangerously-bypass-approvals-and-sandbox exec --skip-git-repo-check --model gpt-5.5 --config model_reasoning_effort="low""#,
        ),
        (
            "claude:opus:low",
            Headless,
            "--dangerously-skip-permissions -p --model opus --effort low",
        ),
        (
            "pi:tencent/hy3:low",
            Headless,
            "-p --model tencent/hy3 --thinking low",
        ),
    ] {
        let invocation = invocation(spec, defaults, &models()).unwrap();
        if invocation.spec.family() == "claude" && !matches!(defaults, Headless) {
            let adapter = invocation.spec.profile().acp.unwrap().adapter;
            let session = adapter
                .configure(&invocation.spec, "instructions", matches!(defaults, Worker))
                .unwrap();
            assert_eq!(
                session.mode.unwrap().to_string(),
                if matches!(defaults, Foreground) {
                    "default"
                } else {
                    "bypassPermissions"
                }
            );
            let meta = session.meta.unwrap();
            let options = meta.get("claudeCode").unwrap().get("options").unwrap();
            for (key, value) in [
                ("model", invocation.spec.model()),
                ("effort", invocation.spec.effort()),
            ] {
                assert_eq!(
                    options.get(key),
                    value.map(serde_json::Value::from).as_ref()
                );
            }
            assert_eq!(
                meta.get("systemPrompt").unwrap().get("append").unwrap(),
                "instructions"
            );
        }
        assert_eq!(invocation.binary, invocation.spec.family(), "{spec}");
        assert_eq!(
            invocation.args,
            args.split_whitespace().collect::<Vec<_>>(),
            "{spec}"
        );
    }
}

#[test]
fn pi_acp_wrapper_preserves_rpc_arguments_and_pins_subscription_configuration() {
    use crate::test_support;
    use camino::Utf8Path;
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let dir = Utf8Path::from_path(temp.path()).unwrap();
    let profile = profile_for("pi").unwrap().acp.unwrap();
    let adapter = profile.adapter;
    for spec in ["pi", "pi:tencent/hy3:low"] {
        assert!(
            adapter
                .configure(&AgentSpec::parse(spec, &models()).unwrap(), "", true)
                .is_err()
        );
    }
    let spec = AgentSpec::parse("pi:openai/gpt-6.1-sol:high", &models()).unwrap();
    let primary = "instructions 'quoted'\n$(false)";
    let mut config = adapter.configure(&spec, primary, true).unwrap();
    assert!(
        serde_json::to_string(&config)
            .unwrap_err()
            .to_string()
            .contains("prepare")
    );
    config.prepare(dir).unwrap();
    test_support::write_executable_script(&dir.join("pi"), "#!/bin/sh\nprintf '%s\n' \"$@\"\n");
    let extra = ["--session", "saved conversation.jsonl"];
    let output = Command::new(&config.env.first().unwrap().1)
        .env("PATH", dir)
        .args(["--mode", "rpc", "--no-themes"])
        .args(extra)
        .output()
        .unwrap();
    assert!(output.status.success());
    let mut expected = vec!["--mode", "rpc", "--no-themes"];
    expected.extend(extra);
    let extension = dir.join("acp-extension.js");
    expected.extend([
        "--model",
        "openai/gpt-6.1-sol",
        "--thinking",
        "high",
        "--provider",
        "openai",
        "--append-system-prompt",
        primary,
        "--extension",
        extension.as_str(),
    ]);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{}\n", expected.join("\n"))
    );
    assert_eq!(config.env.first().unwrap().0, "PI_ACP_PI_COMMAND");
    let update = serde_json::from_value(serde_json::json!({
        "toolCallId": "owned",
        "_meta": {"terminal_output": {"data": "owned output"}}
    }))
    .unwrap();
    assert_eq!(profile.terminal_output(&update), Some("owned output"));
}

#[test]
fn pi_subscription_provider_keeps_oauth_and_rejects_api_keys() {
    use crate::store::atomic_write;
    use std::{fs, process::Command};
    let root = tempfile::tempdir().unwrap();
    for (path, body) in [
        ("package.json", r#"{"type":"module"}"#),
        (
            "node_modules/@earendil-works/pi-ai/package.json",
            r#"{"type":"module","exports":{"./providers/all":"./all.js"}}"#,
        ),
        (
            "node_modules/@earendil-works/pi-ai/all.js",
            r#"export const oauth={isSubscription:true};
export const builtinProviders=()=>[{id:'openai',auth:{oauth,apiKey:{resolve:async()=>({auth:{apiKey:'fixture'}})}}}];"#,
        ),
        ("guard.js", include_str!("pi-subscription.js")),
    ] {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        atomic_write(path, body).unwrap();
    }
    let output = Command::new("node").args(["--input-type=module", "-e", r#"
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
const {default:guard}=await import(pathToFileURL(process.argv[1]));
let provider;
guard({registerProvider:p=>provider=p});
assert.equal(provider.auth.oauth.isSubscription,true);
await assert.rejects(provider.auth.apiKey.resolve({credential:{type:'api_key',key:'fixture'}}),/API-key billing refused/);
"#]).arg(root.path().join("guard.js")).output().unwrap();
    assert!(output.status.success(), "{output:?}");
}
