use std::{
    fs,
    process::{Command, Stdio},
};

use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::{
    config::{agents, version},
    util::write_json_pretty,
};

pub fn analyze(agent: Option<String>) -> Result<()> {
    let agents = match agent {
        Some(agent) => vec![agent],
        None => agents::known_agent_ids().map(str::to_owned).collect(),
    };

    let dir = Utf8Path::new(".niles").join("capabilities");
    fs::create_dir_all(&dir).context("failed to create capability directory")?;

    for agent in agents {
        let spec = agents::parse_spec(&agent)?;
        let binary = agents::default_binary(spec.family());
        let manifest = probe_agent(&spec, &binary);
        if let Some(gate) = &manifest.version_gate {
            println!("{}", gate.status_line());
        }
        let path = dir.join(format!("{agent}.json"));
        write_json_pretty(&path, &manifest)?;
        println!("wrote {path}");
    }

    Ok(())
}

#[derive(Debug, Serialize)]
struct CapabilityManifest {
    agent: String,
    binary: String,
    analyzed_at: DateTime<Utc>,
    version_probe: ProbeResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    version_gate: Option<version::VersionGateReport>,
    help_probe: ProbeResult,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProbeResult {
    pub(crate) status: ProbeStatus,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProbeStatus {
    Success,
    Failed,
    NotFound,
}

fn probe_agent(agent: &agents::AgentSpec, binary: &str) -> CapabilityManifest {
    let version_probe = run_probe(binary, "--version");
    let version_gate = version::evaluate_agent_probe(agent.family(), binary, &version_probe);
    CapabilityManifest {
        agent: agent.original().to_owned(),
        binary: binary.to_owned(),
        analyzed_at: Utc::now(),
        version_probe,
        version_gate,
        help_probe: run_probe(binary, "--help"),
    }
}

pub(crate) fn run_probe(binary: &str, arg: &str) -> ProbeResult {
    let output = Command::new(binary).arg(arg).stdin(Stdio::null()).output();

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
