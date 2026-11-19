use super::*;

#[test]
fn parses_agent_model_effort_specs() {
    let max = Some("MAX");
    for (input, family, model, effort) in [
        ("pi:provider/model:free:", "pi", "provider/model:free", None),
        (
            "pi:provider/model:free:future",
            "pi",
            "provider/model:free",
            Some("future"),
        ),
        ("codex:gpt-5.5:xhigh", "codex", "gpt-5.5", Some("xhigh")),
        ("claude:sonnet:med", "claude", "sonnet", Some("med")),
        ("Claude:Opus:MAX", "claude", "Opus", max),
        ("Claude:Haiku", "claude", "Haiku", None),
        ("CODEX:GPT-5.6-LUNA:MAX", "codex", "GPT-5.6-LUNA", max),
    ] {
        let spec = AgentSpec::parse(input).unwrap();
        assert_eq!(
            (spec.family(), spec.model(), spec.effort()),
            (family, Some(model), effort),
            "{input}"
        );
        assert_eq!(AgentSpec::parse(&spec.canonical()).unwrap(), spec);
    }
    for spec in ["codex::high", "claude:"] {
        assert!(AgentSpec::parse(spec).is_err(), "{spec}");
    }
    assert!(AgentSpec::from_parts("codex", None, Some("high")).is_err());
}

#[test]
fn only_supported_families_are_launchable() {
    assert_eq!(
        known_agent_ids().collect::<Vec<_>>(),
        ["codex", "claude", "pi"]
    );
    for spec in ["hermes", "hermes:z-ai/glm-5.3-flash:high"] {
        let error = invocation(spec, InvocationDefaults::Worker).unwrap_err();
        assert!(error.to_string().contains("unknown agent family `hermes`"));
    }
}

#[test]
fn invocations_map_models_efforts_and_order_headless_defaults() {
    use InvocationDefaults::{Foreground, Headless, Worker};
    for (spec, defaults, args) in [
        ("claude", Foreground, ""),
        (
            "codex:future-model:future-effort",
            Worker,
            r#"--no-daemon --dangerously-bypass-approvals-and-sandbox --model future-model --config model_reasoning_effort="future-effort""#,
        ),
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
        let invocation = invocation(spec, defaults).unwrap();
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
                .configure(&AgentSpec::parse(spec).unwrap(), "", true)
                .is_err()
        );
    }
    let spec = AgentSpec::parse("pi:openai/gpt-6.1-sol:high").unwrap();
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
