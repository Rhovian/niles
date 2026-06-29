use std::{
    collections::BTreeMap,
    fs,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use clap::{ArgAction, Parser, Subcommand};
use serde::{Deserialize, Serialize};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        CommandName::Ask { agent, prompt } => ask(agent, prompt),
        CommandName::Analyze { agent } => analyze(agent),
        CommandName::Run { task } => run(task),
        CommandName::Resume { run } => resume(run),
        CommandName::Status { run } => status(run),
    }
}

#[derive(Debug, Parser)]
#[command(
    version,
    about,
    infer_subcommands = true,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: CommandName,
}

#[derive(Debug, Subcommand)]
enum CommandName {
    /// Start a one-off agent task without writing YAML.
    #[command(alias = "a")]
    Ask {
        /// Agent id to hand the task to.
        #[arg(short, long, default_value = "codex")]
        agent: String,
        /// Prompt to send to the agent.
        #[arg(required = true, action = ArgAction::Append, trailing_var_arg = true)]
        prompt: Vec<String>,
    },
    /// Probe configured agent CLIs and write local capability manifests.
    #[command(alias = "doctor", alias = "scan")]
    Analyze {
        /// Agent id to probe. Defaults to codex and claude.
        #[arg(short, long)]
        agent: Option<String>,
    },
    /// Start a new run from a task spec.
    #[command(alias = "r")]
    Run {
        /// YAML task specification.
        task: Utf8PathBuf,
    },
    /// Resume a persisted run.
    #[command(alias = "re")]
    Resume {
        /// Run id or "latest".
        #[arg(default_value = "latest")]
        run: String,
    },
    /// Inspect a persisted run.
    #[command(alias = "s")]
    Status {
        /// Run id or "latest".
        #[arg(default_value = "latest")]
        run: String,
    },
}

#[derive(Debug, Deserialize)]
struct TaskSpec {
    goal: String,
    #[serde(default)]
    agents: BTreeMap<String, AgentConfig>,
    #[serde(default)]
    steps: Vec<Step>,
    #[serde(default)]
    commands: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct AgentConfig {
    binary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Step {
    Agent { agent: String, task: String },
    Command { command: String },
}

#[derive(Debug, Serialize)]
struct RunState {
    id: String,
    goal: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_file: Option<Utf8PathBuf>,
    created_at: DateTime<Utc>,
    status: RunStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum RunStatus {
    Created,
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

fn ask(agent: String, prompt: Vec<String>) -> Result<()> {
    let prompt = prompt.join(" ");
    let spec = TaskSpec {
        goal: prompt.clone(),
        agents: BTreeMap::from([(agent.clone(), AgentConfig { binary: None })]),
        steps: vec![Step::Agent {
            agent,
            task: prompt,
        }],
        commands: BTreeMap::new(),
    };

    create_run(spec, None)
}

fn analyze(agent: Option<String>) -> Result<()> {
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

fn run(task: Utf8PathBuf) -> Result<()> {
    let body = fs::read_to_string(&task).with_context(|| format!("failed to read {task}"))?;
    let spec: TaskSpec = serde_yaml::from_str(&body).context("failed to parse task YAML")?;
    create_run(spec, Some(task))
}

fn create_run(spec: TaskSpec, task_file: Option<Utf8PathBuf>) -> Result<()> {
    if spec.steps.is_empty() {
        bail!("task spec must contain at least one step");
    }

    let now = Utc::now();
    let id = format!(
        "{}{:09}Z",
        now.format("%Y%m%dT%H%M%S"),
        now.timestamp_subsec_nanos()
    );
    let run_dir = Utf8Path::new(".niles").join("runs").join(&id);
    fs::create_dir_all(&run_dir).with_context(|| format!("failed to create {run_dir}"))?;
    let plan = summarize_spec(&spec);

    let state = RunState {
        id: id.clone(),
        goal: spec.goal,
        task_file,
        created_at: now,
        status: RunStatus::Created,
    };

    let state_path = run_dir.join("state.json");
    fs::write(&state_path, serde_json::to_string_pretty(&state)?)
        .with_context(|| format!("failed to write {state_path}"))?;

    let plan_path = run_dir.join("plan.json");
    fs::write(&plan_path, serde_json::to_string_pretty(&plan)?)
        .with_context(|| format!("failed to write {plan_path}"))?;

    println!("run: {id}");
    println!("state: {state_path}");
    println!("status: created");
    println!("next: workflow execution is not implemented yet");

    Ok(())
}

fn resume(run: String) -> Result<()> {
    let run_dir = resolve_run_dir(&run)?;
    println!("resume target: {run_dir}");
    println!("next: resume execution is not implemented yet");
    Ok(())
}

fn status(run: String) -> Result<()> {
    let run_dir = resolve_run_dir(&run)?;
    let state_path = run_dir.join("state.json");
    let state =
        fs::read_to_string(&state_path).with_context(|| format!("failed to read {state_path}"))?;
    println!("{state}");
    Ok(())
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

fn resolve_run_dir(run: &str) -> Result<Utf8PathBuf> {
    let runs_dir = Utf8Path::new(".niles").join("runs");

    if run != "latest" {
        return Ok(runs_dir.join(run));
    }

    let mut runs = fs::read_dir(&runs_dir)
        .with_context(|| format!("failed to read {runs_dir}"))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = Utf8PathBuf::from_path_buf(entry.path()).ok()?;
            path.is_dir().then_some(path)
        })
        .collect::<Vec<_>>();

    runs.sort();
    runs.pop().context("no runs found")
}

fn summarize_spec(spec: &TaskSpec) -> serde_json::Value {
    let agents = spec
        .agents
        .iter()
        .map(|(id, config)| {
            serde_json::json!({
                "id": id,
                "binary": config.binary.as_deref().unwrap_or(id),
            })
        })
        .collect::<Vec<_>>();

    let steps = spec
        .steps
        .iter()
        .map(|step| match step {
            Step::Agent { agent, task } => {
                serde_json::json!({ "agent": agent, "task": task })
            }
            Step::Command { command } => serde_json::json!({ "command": command }),
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "agents": agents,
        "steps": steps,
        "commands": spec.commands,
    })
}
