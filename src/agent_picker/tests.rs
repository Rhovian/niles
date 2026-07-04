use super::*;

use std::{
    fs,
    io::Cursor,
    os::unix::fs::PermissionsExt,
    time::{SystemTime, UNIX_EPOCH},
};

use camino::Utf8PathBuf;
use chrono::Utc;

use crate::{
    analyze::{ProbeResult, ProbeStatus},
    capabilities::{self, CapabilityManifest, ModelProbe},
    config::{
        spec::PromptMode,
        version::{VersionGateReport, VersionGateStatus},
    },
    schema,
};

#[test]
fn picker_uses_static_catalog_when_fresh_manifest_is_unavailable() {
    let root = temp_test_path("static-catalog");
    fs::create_dir_all(&root).unwrap();
    let mut input = Cursor::new(b"1\n3\n5\n".to_vec());
    let mut output = Vec::new();

    let value = prompt_agent_value(
        &root,
        &mut input,
        &mut output,
        "Manager agent",
        "claude",
        &BTreeMap::new(),
    )
    .unwrap();

    assert_eq!(value, "codex:o3-pro:high");
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Manager agent:"));
    assert!(output.contains("Select Manager agent [claude]: "));
    assert!(output.contains("codex model options: static catalog"));
    assert!(output.contains("run `niles analyze --agent codex`"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn picker_uses_fresh_capability_manifest_for_model_versions_and_effort() {
    let root = temp_test_path("fresh-catalog");
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let binary = bin.join("claude");
    write_executable(
        &binary,
        r#"#!/bin/sh
case "$1" in
  --version) printf '2.1.197 (Claude Code)\n'; exit 0 ;;
  --help) printf 'claude help\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );
    let binary = binary.to_string();
    let mut configs = BTreeMap::new();
    configs.insert(
        "claude".to_owned(),
        AgentConfig {
            binary: Some(binary.clone()),
            args: Vec::new(),
            prompt: PromptMode::Arg,
        },
    );
    write_capability_manifest(
        &root,
        "claude",
        &binary,
        vec![
            model_probe("opus", None),
            model_probe("claude-opus-4-5-20251001", Some("max")),
        ],
    );

    let mut input = Cursor::new(b"2\n2\n2\n2\n".to_vec());
    let mut output = Vec::new();
    let value = prompt_agent_value(
        &root,
        &mut input,
        &mut output,
        "Reviewer agent",
        "codex",
        &configs,
    )
    .unwrap();

    assert_eq!(value, "claude:claude-opus-4-5-20251001:max");
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("claude model options: probed from"));
    assert!(output.contains("claude opus version:"));
    assert!(output.contains("Select claude effort [1]: "));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn picker_other_choice_validates_and_canonicalizes_free_text_specs() {
    let root = temp_test_path("other-choice");
    fs::create_dir_all(&root).unwrap();
    let mut input = Cursor::new(b"3\nClaude:Sonnet:med\n".to_vec());
    let mut output = Vec::new();

    let value = prompt_agent_value(
        &root,
        &mut input,
        &mut output,
        "Planner agent",
        "codex",
        &BTreeMap::new(),
    )
    .unwrap();

    assert_eq!(value, "claude:sonnet:medium");
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("other..."));
    assert!(output.contains("Custom agent spec [codex]: "));

    fs::remove_dir_all(root).unwrap();
}

fn write_capability_manifest(
    root: &camino::Utf8Path,
    family: &str,
    binary: &str,
    accepted_models: Vec<ModelProbe>,
) {
    let now = Utc::now();
    let manifest = CapabilityManifest {
        agent: family.to_owned(),
        binary: binary.to_owned(),
        analyzed_at: now,
        version_probe: ProbeResult {
            status: ProbeStatus::Success,
            stdout: "2.1.197 (Claude Code)\n".to_owned(),
            stderr: String::new(),
        },
        version_gate: Some(VersionGateReport {
            agent: family.to_owned(),
            binary: binary.to_owned(),
            status: VersionGateStatus::Pass,
            min_version: "2.1.197".to_owned(),
            tested_version: "2.1.197".to_owned(),
            detected_version: Some("2.1.197".to_owned()),
            message: "pass".to_owned(),
        }),
        help_probe: ProbeResult {
            status: ProbeStatus::Success,
            stdout: "claude help\n".to_owned(),
            stderr: String::new(),
        },
        accepted_models,
        rejected_models: Vec::new(),
    };
    let path = capabilities::manifest_path_for_binary(root, family, binary);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    schema::write_json(&path, &manifest).unwrap();
}

fn model_probe(model: &str, effort: Option<&str>) -> ModelProbe {
    ModelProbe {
        model: model.to_owned(),
        effort: effort.map(str::to_owned),
        cli_version: Some("2.1.197".to_owned()),
        probed_at: Utc::now(),
        stdout: String::new(),
        stderr: String::new(),
    }
}

fn write_executable(path: &camino::Utf8Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn temp_test_path(label: &str) -> Utf8PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
        "niles-agent-picker-{label}-{}-{nanos}",
        std::process::id()
    )))
    .unwrap()
}
