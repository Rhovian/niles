use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    process::{Command, Output, Stdio},
};

use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    agents::{
        self,
        capabilities::{CapabilityManifest, ModelProbe, manifest_path_for_binary},
        version,
    },
    config::spec::{AgentConfig, load_project_config},
    util::write_json_pretty,
    workspace_manifest,
};

const MODEL_PROBE_PROMPT: &str = "Reply with OK only.";

pub fn analyze(requested_agent: Option<String>) -> Result<()> {
    let config = load_project_config()?;
    let agents = agents_to_probe(requested_agent, &config.agents)?;

    let dir = Utf8Path::new(".niles").join("capabilities");
    fs::create_dir_all(&dir).context("failed to create capability directory")?;

    for (target, specs) in agents {
        let manifest = probe_agent(&target.family, &specs, &target.binary)?;
        if let Some(gate) = &manifest.version_gate {
            println!("{}", gate.status_line());
        }
        print_model_probe_lines(&target.family, &manifest);
        let path = manifest_path_for_binary(Utf8Path::new(""), &target.family, &target.binary);
        write_json_pretty(&path, &manifest)?;
        println!("wrote {path}");
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ProbeTarget {
    family: String,
    binary: String,
}

#[derive(Debug, Clone)]
struct ProbeSpec {
    spec: agents::AgentSpec,
    config: Option<AgentConfig>,
}

fn agents_to_probe(
    requested_agent: Option<String>,
    configs: &BTreeMap<String, AgentConfig>,
) -> Result<BTreeMap<ProbeTarget, Vec<ProbeSpec>>> {
    let mut specs = BTreeMap::new();
    match requested_agent {
        Some(requested_agent) => {
            let spec = agents::parse_spec(&requested_agent)?;
            let family = spec.family().to_owned();
            let probes_aliases = spec.model().is_none();
            insert_spec(&mut specs, spec, configs)?;
            if probes_aliases {
                for model in agents::default_model_aliases(&family) {
                    insert_spec(
                        &mut specs,
                        agents::AgentSpec::from_parts(&family, Some(model), None)?,
                        configs,
                    )?;
                }
            }
        }
        None => insert_default_analyze_specs(&mut specs, configs)?,
    }
    Ok(specs)
}

fn insert_default_analyze_specs(
    specs: &mut BTreeMap<ProbeTarget, Vec<ProbeSpec>>,
    configs: &BTreeMap<String, AgentConfig>,
) -> Result<()> {
    for family in agents::known_agent_ids() {
        insert_spec(specs, agents::parse_spec(family)?, configs)?;
        for model in agents::default_model_aliases(family) {
            insert_spec(
                specs,
                agents::AgentSpec::from_parts(family, Some(model), None)?,
                configs,
            )?;
        }
    }
    insert_workspace_manifest_specs(specs, configs)
}

fn insert_workspace_manifest_specs(
    specs: &mut BTreeMap<ProbeTarget, Vec<ProbeSpec>>,
    configs: &BTreeMap<String, AgentConfig>,
) -> Result<()> {
    let Some(manifest) = workspace_manifest::load(Utf8Path::new("."))? else {
        return Ok(());
    };
    for agent in [
        manifest.manager,
        manifest.planner,
        manifest.worker,
        manifest.reviewer,
    ] {
        insert_spec(specs, agents::parse_spec(&agent)?, configs)?;
    }
    Ok(())
}

fn insert_spec(
    specs: &mut BTreeMap<ProbeTarget, Vec<ProbeSpec>>,
    spec: agents::AgentSpec,
    configs: &BTreeMap<String, AgentConfig>,
) -> Result<()> {
    let key = spec_key(&spec);
    let agent = spec.canonical();
    let config = agents::config_for(configs, &agent)?.cloned();
    let invocation =
        agents::invocation(&agent, config.as_ref(), agents::InvocationDefaults::Default)?;
    let target = ProbeTarget {
        family: spec.family().to_owned(),
        binary: invocation.binary,
    };
    let entries = specs.entry(target).or_default();
    if !entries
        .iter()
        .any(|existing| spec_key(&existing.spec) == key)
    {
        entries.push(ProbeSpec { spec, config });
    }
    Ok(())
}

fn spec_key(spec: &agents::AgentSpec) -> (Option<&str>, Option<&str>) {
    (spec.model(), spec.effort())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProbeResult {
    pub(crate) status: ProbeStatus,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProbeStatus {
    Success,
    Failed,
    NotFound,
}

fn probe_agent(family: &str, specs: &[ProbeSpec], binary: &str) -> Result<CapabilityManifest> {
    let version_probe = run_probe(binary, "--version");
    let version_gate = version::evaluate_agent_probe(family, binary, &version_probe);
    let (accepted_models, rejected_models) =
        probe_model_acceptance(specs, binary, version_gate.as_ref())?;
    Ok(CapabilityManifest {
        agent: family.to_owned(),
        binary: binary.to_owned(),
        analyzed_at: Utc::now(),
        accepted_models,
        rejected_models,
        version_probe,
        version_gate,
        help_probe: run_probe(binary, "--help"),
    })
}

fn probe_model_acceptance(
    specs: &[ProbeSpec],
    binary: &str,
    version_gate: Option<&version::VersionGateReport>,
) -> Result<(Vec<ModelProbe>, Vec<ModelProbe>)> {
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    let mut seen_specs = BTreeSet::new();
    for target in specs {
        let Some(model) = target.spec.model() else {
            continue;
        };
        let key = (model.to_owned(), target.spec.effort().map(str::to_owned));
        if !seen_specs.insert(key) {
            continue;
        }
        let probe = run_model_probe(target, binary)?;
        let model_probe = ModelProbe {
            model: model.to_owned(),
            effort: target.spec.effort().map(str::to_owned),
            cli_version: version_gate.and_then(|gate| gate.detected_version.clone()),
            probed_at: Utc::now(),
            stdout: probe.stdout,
            stderr: probe.stderr,
        };
        match probe.status {
            ProbeStatus::Success => accepted.push(model_probe),
            ProbeStatus::Failed => rejected.push(model_probe),
            ProbeStatus::NotFound => {}
        }
    }
    Ok((accepted, rejected))
}

fn run_model_probe(target: &ProbeSpec, binary: &str) -> Result<ProbeResult> {
    let invocation = agents::invocation(
        &target.spec.canonical(),
        target.config.as_ref(),
        agents::InvocationDefaults::Default,
    )?;
    let mut args = invocation.args;
    match invocation.prompt {
        crate::config::spec::PromptMode::Arg => {
            args.push(MODEL_PROBE_PROMPT.to_owned());
            Ok(run_probe_args(binary, &args))
        }
        crate::config::spec::PromptMode::Stdin => Ok(output_to_probe_result(run_probe_with_stdin(
            binary,
            &args,
            MODEL_PROBE_PROMPT,
        ))),
    }
}

fn print_model_probe_lines(family: &str, manifest: &CapabilityManifest) {
    for probe in &manifest.accepted_models {
        println!("{}", model_probe_line(family, probe, "accepted"));
    }
    for probe in &manifest.rejected_models {
        println!("{}", model_probe_line(family, probe, "not accepted"));
    }
}

fn model_probe_line(family: &str, probe: &ModelProbe, status: &str) -> String {
    let effort = match probe.effort.as_deref() {
        Some(effort) => format!(":{effort}"),
        None => String::new(),
    };
    let version = match probe.cli_version.as_deref() {
        Some(version) => version,
        None => "version unavailable",
    };
    format!(
        "model_probe: {family}:{}{effort} {status} by {family} CLI {version}",
        probe.model
    )
}

pub(crate) fn run_probe(binary: &str, arg: &str) -> ProbeResult {
    run_probe_args(binary, &[arg.to_owned()])
}

fn run_probe_args(binary: &str, args: &[String]) -> ProbeResult {
    output_to_probe_result(
        Command::new(binary)
            .args(args)
            .stdin(Stdio::null())
            .output(),
    )
}

fn run_probe_with_stdin(binary: &str, args: &[String], stdin: &str) -> std::io::Result<Output> {
    let mut child = Command::new(binary)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut child_stdin = child
        .stdin
        .take()
        .context("failed to open probe stdin pipe")
        .map_err(std::io::Error::other)?;
    child_stdin.write_all(stdin.as_bytes())?;
    drop(child_stdin);
    child.wait_with_output()
}

fn output_to_probe_result(output: std::io::Result<Output>) -> ProbeResult {
    match output {
        Ok(output) => ProbeResult {
            status: if output.status.success() {
                ProbeStatus::Success
            } else {
                ProbeStatus::Failed
            },
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => ProbeResult {
            status: ProbeStatus::NotFound,
            stdout: String::new(),
            stderr: err.to_string(),
        },
        Err(err) => ProbeResult {
            status: ProbeStatus::Failed,
            stdout: String::new(),
            stderr: err.to_string(),
        },
    }
}
