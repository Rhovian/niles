use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    agents::{self, AgentSpec, InvocationDefaults, version},
    analyze::{ProbeResult, run_probe},
    config::spec::{AgentConfig, TaskSpec, TaskStep},
    schema::{self, ArtifactKind},
    util::slugify,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CapabilityManifest {
    pub(crate) agent: String,
    pub(crate) binary: String,
    pub(crate) analyzed_at: DateTime<Utc>,
    pub(crate) version_probe: ProbeResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) version_gate: Option<version::VersionGateReport>,
    pub(crate) help_probe: ProbeResult,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) accepted_models: Vec<ModelProbe>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) rejected_models: Vec<ModelProbe>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ModelProbe {
    pub(crate) model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cli_version: Option<String>,
    pub(crate) probed_at: DateTime<Utc>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

#[derive(Debug, Clone)]
pub(crate) struct FreshAcceptedModels {
    pub(crate) path: Utf8PathBuf,
    pub(crate) accepted_models: Vec<ModelProbe>,
}

pub(crate) fn manifest_path(workspace: &Utf8Path, family: &str) -> Utf8PathBuf {
    workspace
        .join(".niles")
        .join("capabilities")
        .join(format!("{family}.json"))
}

pub(crate) fn manifest_path_for_binary(
    workspace: &Utf8Path,
    family: &str,
    binary: &str,
) -> Utf8PathBuf {
    if binary == agents::default_binary(family) {
        manifest_path(workspace, family)
    } else {
        workspace.join(".niles").join("capabilities").join(format!(
            "{family}-{}-{}.json",
            slugify(binary),
            stable_hash(binary)
        ))
    }
}

fn stable_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub(crate) fn validate_task_agents(spec: &TaskSpec) -> Result<()> {
    let workspace = spec
        .workspace
        .as_deref()
        .context("run spec has no resolved workspace")?;

    for step in &spec.steps {
        if let TaskStep::Agent { agent, .. } = step {
            validate_task_agent(workspace, &spec.agents, agent)?;
        }
    }

    Ok(())
}

pub(crate) fn validate_task_agent(
    workspace: &Utf8Path,
    configs: &BTreeMap<String, AgentConfig>,
    agent: &str,
) -> Result<AgentSpec> {
    let config = agents::config_for(configs, agent)?;
    validate_agent(workspace, agent, config, InvocationDefaults::Default)
}

pub(crate) fn validate_agent(
    workspace: &Utf8Path,
    agent: &str,
    config: Option<&AgentConfig>,
    defaults: InvocationDefaults,
) -> Result<AgentSpec> {
    let spec = agents::parse_spec(agent)?;
    validate_model(workspace, &spec, config, defaults)?;
    Ok(spec)
}

pub(crate) fn fresh_accepted_models(
    workspace: &Utf8Path,
    family: &str,
    config: Option<&AgentConfig>,
    defaults: InvocationDefaults,
) -> Result<Option<FreshAcceptedModels>> {
    let invocation = agents::invocation(family, config, defaults)?;
    let Some((path, manifest)) = read_manifest_for_binary(workspace, family, &invocation.binary)?
    else {
        return Ok(None);
    };

    if !manifest_matches_binary(&manifest, &invocation.binary) {
        return Ok(None);
    }

    let current = current_cli_version(family, &invocation.binary)?;
    if !manifest_is_fresh(&manifest, current.as_deref()) {
        return Ok(None);
    }

    Ok(Some(FreshAcceptedModels {
        path,
        accepted_models: manifest.accepted_models,
    }))
}

fn validate_model(
    workspace: &Utf8Path,
    spec: &AgentSpec,
    config: Option<&AgentConfig>,
    defaults: InvocationDefaults,
) -> Result<()> {
    let Some(model) = spec.model() else {
        return Ok(());
    };

    let invocation = agents::invocation(&spec.canonical(), config, defaults)?;
    let Some((path, manifest)) =
        read_manifest_for_binary(workspace, spec.family(), &invocation.binary)?
    else {
        return agents::validate_static_model(spec);
    };

    if !manifest_matches_binary(&manifest, &invocation.binary) {
        warn_stale_manifest(&path, spec.family(), &manifest, None);
        return agents::validate_static_model(spec);
    }

    let current = current_cli_version(spec.family(), &invocation.binary)?;
    if !manifest_is_fresh(&manifest, current.as_deref()) {
        warn_stale_manifest(&path, spec.family(), &manifest, current.as_deref());
        return agents::validate_static_model(spec);
    }

    let effort = spec.effort();
    if model_probe_matches(&manifest.accepted_models, model, effort) {
        return Ok(());
    }
    if let Some(probe) = matching_model_probe(&manifest.rejected_models, model, effort) {
        let version = probe
            .cli_version
            .as_deref()
            .or_else(|| manifest_cli_version(&manifest))
            .map_or(version::VERSION_UNAVAILABLE_PLACEHOLDER, |version| version);
        bail!(
            "model `{model}` was rejected by {} CLI {version} in {path}; rerun `niles analyze`, or check the model name",
            spec.family()
        );
    }

    agents::validate_static_model(spec)
}

fn read_manifest_for_binary(
    workspace: &Utf8Path,
    family: &str,
    binary: &str,
) -> Result<Option<(Utf8PathBuf, CapabilityManifest)>> {
    let exact_path = manifest_path_for_binary(workspace, family, binary);
    if let Some(manifest) =
        schema::read_optional_json(&exact_path, ArtifactKind::CapabilityManifest)?
    {
        return Ok(Some((exact_path, manifest)));
    }

    let legacy_path = manifest_path(workspace, family);
    if legacy_path != exact_path
        && let Some(manifest) =
            schema::read_optional_json(&legacy_path, ArtifactKind::CapabilityManifest)?
    {
        return Ok(Some((legacy_path, manifest)));
    }

    Ok(None)
}

fn current_cli_version(family: &str, binary: &str) -> Result<Option<String>> {
    let probe = run_probe(binary, "--version");
    let report = version::evaluate_agent_probe(family, binary, &probe)?
        .with_context(|| format!("agent family `{family}` has no version gate"))?;
    Ok(report.detected_version)
}

fn manifest_matches_binary(manifest: &CapabilityManifest, binary: &str) -> bool {
    manifest.binary == binary
}

fn manifest_is_fresh(manifest: &CapabilityManifest, current: Option<&str>) -> bool {
    manifest_cli_version(manifest).is_some() && manifest_cli_version(manifest) == current
}

fn manifest_cli_version(manifest: &CapabilityManifest) -> Option<&str> {
    manifest
        .version_gate
        .as_ref()
        .and_then(|gate| gate.detected_version.as_deref())
}

fn warn_stale_manifest(
    path: &Utf8Path,
    family: &str,
    manifest: &CapabilityManifest,
    current: Option<&str>,
) {
    let manifest_version = manifest_cli_version(manifest)
        .map_or(version::VERSION_UNAVAILABLE_PLACEHOLDER, |version| version);
    let current_version =
        current.map_or(version::VERSION_UNAVAILABLE_PLACEHOLDER, |version| version);
    eprintln!(
        "warning: capability manifest {path} was probed for {family} CLI {manifest_version}, but the current CLI is {current_version}; rerun `niles analyze` to refresh model acceptance data"
    );
}

fn model_probe_matches(probes: &[ModelProbe], model: &str, effort: Option<&str>) -> bool {
    matching_model_probe(probes, model, effort).is_some()
}

fn matching_model_probe<'a>(
    probes: &'a [ModelProbe],
    model: &str,
    effort: Option<&str>,
) -> Option<&'a ModelProbe> {
    probes
        .iter()
        .find(|probe| probe.model == model && probe.effort.as_deref() == effort)
}
