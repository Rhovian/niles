use std::{
    fs,
    process::{Command, Stdio},
};

use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, Utc};
use serde::Serialize;

pub fn analyze(agent: Option<String>) -> Result<()> {
    let agents = match agent {
        Some(agent) => vec![agent],
        None => vec!["codex".to_owned(), "claude".to_owned()],
    };

    let dir = Utf8Path::new(".niles").join("capabilities");
    fs::create_dir_all(&dir).context("failed to create capability directory")?;

    for agent in agents {
        let manifest = probe_agent(&agent, &agent);
        let path = dir.join(format!("{agent}.json"));
        let body = serde_json::to_string_pretty(&manifest)?;
        fs::write(&path, body).with_context(|| format!("failed to write {path}"))?;
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
    help_probe: ProbeResult,
}

#[derive(Debug, Serialize)]
struct ProbeResult {
    status: ProbeStatus,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProbeStatus {
    Success,
    Failed,
    NotFound,
}

fn probe_agent(agent: &str, binary: &str) -> CapabilityManifest {
    CapabilityManifest {
        agent: agent.to_owned(),
        binary: binary.to_owned(),
        analyzed_at: Utc::now(),
        version_probe: run_probe(binary, "--version"),
        help_probe: run_probe(binary, "--help"),
    }
}

fn run_probe(binary: &str, arg: &str) -> ProbeResult {
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
