use std::{
    collections::BTreeMap,
    fs,
    io::Write,
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
        CommandName::Show { run } => show(run),
        CommandName::Log {
            run,
            step,
            stderr,
            both,
        } => log(run, step, stderr, both),
        CommandName::Diff { run, step } => diff(run, step),
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
    /// Show a compact summary of a persisted run.
    #[command(alias = "sh")]
    Show {
        /// Run id or "latest".
        #[arg(default_value = "latest")]
        run: String,
    },
    /// Print stdout or stderr for a run step.
    #[command(alias = "l")]
    Log {
        /// Run id or "latest".
        #[arg(default_value = "latest")]
        run: String,
        /// Step number to inspect. Defaults to the last recorded step.
        #[arg(short, long)]
        step: Option<usize>,
        /// Print stderr instead of stdout.
        #[arg(long)]
        stderr: bool,
        /// Print stdout and stderr.
        #[arg(long)]
        both: bool,
    },
    /// Print the git diff captured after a run step.
    #[command(alias = "d")]
    Diff {
        /// Run id or "latest".
        #[arg(default_value = "latest")]
        run: String,
        /// Step number to inspect. Defaults to the last recorded step.
        #[arg(short, long)]
        step: Option<usize>,
    },
}

#[derive(Debug, Deserialize)]
struct TaskSpec {
    goal: String,
    #[serde(default)]
    workspace: Option<Utf8PathBuf>,
    #[serde(default)]
    agents: BTreeMap<String, AgentConfig>,
    #[serde(default)]
    steps: Vec<TaskStep>,
    #[serde(default)]
    commands: BTreeMap<String, CommandConfig>,
}

#[derive(Debug, Deserialize)]
struct AgentConfig {
    binary: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    prompt: PromptMode,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TaskStep {
    Agent { agent: String, task: String },
    Command { command: String },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum CommandConfig {
    Short(String),
    Full { run: String },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PromptMode {
    Arg,
    Stdin,
}

impl Default for PromptMode {
    fn default() -> Self {
        Self::Arg
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct RunState {
    id: String,
    goal: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_file: Option<Utf8PathBuf>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    status: RunStatus,
    steps: Vec<StepRecord>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum RunStatus {
    Created,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Deserialize, Serialize)]
struct StepRecord {
    index: usize,
    kind: StepKind,
    label: String,
    status: StepStatus,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    exit_code: Option<i32>,
    stdout: Utf8PathBuf,
    stderr: Utf8PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diff: Option<Utf8PathBuf>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StepKind {
    Agent,
    Command,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StepStatus {
    Completed,
    Failed,
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
        agents: BTreeMap::from([(
            agent.clone(),
            AgentConfig {
                binary: None,
                args: Vec::new(),
                prompt: PromptMode::Arg,
            },
        )]),
        workspace: None,
        steps: vec![TaskStep::Agent {
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
        goal: spec.goal.clone(),
        task_file,
        created_at: now,
        updated_at: now,
        status: RunStatus::Created,
        steps: Vec::new(),
    };

    let state_path = run_dir.join("state.json");
    write_state(&state_path, &state)?;

    let plan_path = run_dir.join("plan.json");
    fs::write(&plan_path, serde_json::to_string_pretty(&plan)?)
        .with_context(|| format!("failed to write {plan_path}"))?;

    println!("run: {id}");
    println!("state: {state_path}");
    execute_run(&spec, &run_dir, state, &state_path)?;

    Ok(())
}

fn execute_run(
    spec: &TaskSpec,
    run_dir: &Utf8Path,
    mut state: RunState,
    state_path: &Utf8Path,
) -> Result<()> {
    let workspace = spec.workspace.as_deref().unwrap_or(Utf8Path::new("."));
    let steps_dir = run_dir.join("steps");
    fs::create_dir_all(&steps_dir).with_context(|| format!("failed to create {steps_dir}"))?;

    state.status = RunStatus::Running;
    state.updated_at = Utc::now();
    write_state(state_path, &state)?;

    for (index, step) in spec.steps.iter().enumerate() {
        let step_number = index + 1;
        let result = match step {
            TaskStep::Agent { agent, task } => {
                println!("step {step_number}: agent {agent}");
                run_agent_step(step_number, agent, task, spec, workspace, &steps_dir)
            }
            TaskStep::Command { command } => {
                println!("step {step_number}: command {command}");
                run_command_step(step_number, command, spec, workspace, &steps_dir)
            }
        }?;

        let failed = matches!(result.status, StepStatus::Failed);
        state.steps.push(result);
        state.status = if failed {
            RunStatus::Failed
        } else {
            RunStatus::Running
        };
        state.updated_at = Utc::now();
        write_state(state_path, &state)?;

        if failed {
            println!("status: failed");
            bail!("step {step_number} failed");
        }
    }

    state.status = RunStatus::Completed;
    state.updated_at = Utc::now();
    write_state(state_path, &state)?;
    println!("status: completed");

    Ok(())
}

fn run_agent_step(
    step_number: usize,
    agent: &str,
    task: &str,
    spec: &TaskSpec,
    workspace: &Utf8Path,
    steps_dir: &Utf8Path,
) -> Result<StepRecord> {
    let config = agent_invocation(agent, spec.agents.get(agent));
    let mut args = config.args;
    let stdin = match config.prompt {
        PromptMode::Arg => {
            args.push(task.to_owned());
            None
        }
        PromptMode::Stdin => Some(task),
    };

    run_process(
        step_number,
        StepKind::Agent,
        agent,
        &config.binary,
        &args,
        stdin,
        workspace,
        steps_dir,
    )
}

fn run_command_step(
    step_number: usize,
    command: &str,
    spec: &TaskSpec,
    workspace: &Utf8Path,
    steps_dir: &Utf8Path,
) -> Result<StepRecord> {
    let config = spec
        .commands
        .get(command)
        .with_context(|| format!("unknown command `{command}`"))?;
    let command_line = command_config_run(config);
    run_process(
        step_number,
        StepKind::Command,
        command,
        "sh",
        &["-c".to_owned(), command_line.to_owned()],
        None,
        workspace,
        steps_dir,
    )
}

fn run_process(
    step_number: usize,
    kind: StepKind,
    label: &str,
    binary: &str,
    args: &[String],
    stdin: Option<&str>,
    workspace: &Utf8Path,
    steps_dir: &Utf8Path,
) -> Result<StepRecord> {
    let started_at = Utc::now();
    let slug = slugify(label);
    let prefix = format!("{step_number:03}-{slug}");
    let stdout_path = steps_dir.join(format!("{prefix}.stdout.txt"));
    let stderr_path = steps_dir.join(format!("{prefix}.stderr.txt"));
    let diff_path = steps_dir.join(format!("{prefix}.diff"));
    let meta_path = steps_dir.join(format!("{prefix}.json"));

    let mut child = Command::new(binary)
        .args(args)
        .current_dir(workspace)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn `{}`", format_invocation(binary, args)))?;

    if let Some(input) = stdin {
        let mut child_stdin = child.stdin.take().context("failed to open child stdin")?;
        child_stdin
            .write_all(input.as_bytes())
            .context("failed to write child stdin")?;
    }

    let output = child
        .wait_with_output()
        .with_context(|| format!("failed to wait for `{}`", format_invocation(binary, args)))?;
    let finished_at = Utc::now();

    fs::write(&stdout_path, &output.stdout)
        .with_context(|| format!("failed to write {stdout_path}"))?;
    fs::write(&stderr_path, &output.stderr)
        .with_context(|| format!("failed to write {stderr_path}"))?;
    capture_git_diff(workspace, &diff_path)?;

    let record = StepRecord {
        index: step_number,
        kind,
        label: label.to_owned(),
        status: if output.status.success() {
            StepStatus::Completed
        } else {
            StepStatus::Failed
        },
        started_at,
        finished_at,
        exit_code: output.status.code(),
        stdout: stdout_path,
        stderr: stderr_path,
        diff: Some(diff_path),
    };

    fs::write(&meta_path, serde_json::to_string_pretty(&record)?)
        .with_context(|| format!("failed to write {meta_path}"))?;

    Ok(record)
}

fn resume(run: String) -> Result<()> {
    let run_dir = resolve_run_dir(&run)?;
    println!("resume target: {run_dir}");
    println!("next: resume execution is not implemented yet");
    Ok(())
}

fn status(run: String) -> Result<()> {
    let run_dir = resolve_run_dir(&run)?;
    let path = state_path(&run_dir);
    let state = fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
    println!("{state}");
    Ok(())
}

fn show(run: String) -> Result<()> {
    let run_dir = resolve_run_dir(&run)?;
    let state = read_state(&run_dir)?;

    println!("run: {}", state.id);
    println!("status: {}", run_status_label(&state.status));
    println!("goal: {}", state.goal);
    println!("created: {}", state.created_at);
    println!("updated: {}", state.updated_at);

    if state.steps.is_empty() {
        println!("steps: none");
        return Ok(());
    }

    println!("steps:");
    for step in &state.steps {
        println!(
            "  {}. {} {} {}{}",
            step.index,
            step_kind_label(&step.kind),
            step.label,
            step_status_label(&step.status),
            step.exit_code
                .map(|code| format!(" ({code})"))
                .unwrap_or_default()
        );
    }

    Ok(())
}

fn log(run: String, step: Option<usize>, stderr: bool, both: bool) -> Result<()> {
    let run_dir = resolve_run_dir(&run)?;
    let state = read_state(&run_dir)?;
    let record = selected_step(&state, step)?;

    if both {
        print_log_file("stdout", &record.stdout)?;
        print_log_file("stderr", &record.stderr)?;
    } else if stderr {
        print!(
            "{}",
            fs::read_to_string(&record.stderr)
                .with_context(|| { format!("failed to read stderr log {}", record.stderr) })?
        );
    } else {
        print!(
            "{}",
            fs::read_to_string(&record.stdout)
                .with_context(|| { format!("failed to read stdout log {}", record.stdout) })?
        );
    }

    Ok(())
}

fn diff(run: String, step: Option<usize>) -> Result<()> {
    let run_dir = resolve_run_dir(&run)?;
    let state = read_state(&run_dir)?;
    let record = selected_step(&state, step)?;
    let diff = record
        .diff
        .as_ref()
        .with_context(|| format!("step {} has no captured diff", record.index))?;
    print!(
        "{}",
        fs::read_to_string(diff).with_context(|| format!("failed to read diff {diff}"))?
    );
    Ok(())
}

struct AgentInvocation {
    binary: String,
    args: Vec<String>,
    prompt: PromptMode,
}

fn agent_invocation(agent: &str, config: Option<&AgentConfig>) -> AgentInvocation {
    let default_binary = agent.to_owned();
    let default_args = match agent {
        "codex" => vec!["exec".to_owned()],
        "claude" => vec!["-p".to_owned()],
        _ => Vec::new(),
    };

    match config {
        Some(config) => AgentInvocation {
            binary: config.binary.clone().unwrap_or(default_binary),
            args: if config.args.is_empty() {
                default_args
            } else {
                config.args.clone()
            },
            prompt: match config.prompt {
                PromptMode::Arg => PromptMode::Arg,
                PromptMode::Stdin => PromptMode::Stdin,
            },
        },
        None => AgentInvocation {
            binary: default_binary,
            args: default_args,
            prompt: PromptMode::Arg,
        },
    }
}

fn command_config_run(config: &CommandConfig) -> &str {
    match config {
        CommandConfig::Short(run) => run,
        CommandConfig::Full { run } => run,
    }
}

fn write_state(path: &Utf8Path, state: &RunState) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(state)?)
        .with_context(|| format!("failed to write {path}"))
}

fn capture_git_diff(workspace: &Utf8Path, diff_path: &Utf8Path) -> Result<()> {
    let output = Command::new("git")
        .args(["diff", "--no-ext-diff", "--"])
        .current_dir(workspace)
        .stdin(Stdio::null())
        .output();

    match output {
        Ok(output) if output.status.success() => {
            fs::write(diff_path, output.stdout)
                .with_context(|| format!("failed to write {diff_path}"))?;
        }
        Ok(output) => {
            fs::write(diff_path, Vec::<u8>::new())
                .with_context(|| format!("failed to write {diff_path}"))?;
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.trim().is_empty() {
                eprintln!("warning: git diff failed: {}", stderr.trim());
            }
        }
        Err(err) => {
            fs::write(diff_path, Vec::<u8>::new())
                .with_context(|| format!("failed to write {diff_path}"))?;
            eprintln!("warning: git diff failed: {err}");
        }
    }

    Ok(())
}

fn selected_step(state: &RunState, step: Option<usize>) -> Result<&StepRecord> {
    match step {
        Some(step) => state
            .steps
            .iter()
            .find(|record| record.index == step)
            .with_context(|| format!("step {step} not found")),
        None => state.steps.last().context("run has no recorded steps"),
    }
}

fn read_state(run_dir: &Utf8Path) -> Result<RunState> {
    let path = state_path(run_dir);
    let body = fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
    serde_json::from_str(&body).with_context(|| format!("failed to parse {path}"))
}

fn state_path(run_dir: &Utf8Path) -> Utf8PathBuf {
    run_dir.join("state.json")
}

fn print_log_file(label: &str, path: &Utf8Path) -> Result<()> {
    println!("==> {label}: {path} <==");
    print!(
        "{}",
        fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?
    );
    Ok(())
}

fn run_status_label(status: &RunStatus) -> &'static str {
    match status {
        RunStatus::Created => "created",
        RunStatus::Running => "running",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
    }
}

fn step_kind_label(kind: &StepKind) -> &'static str {
    match kind {
        StepKind::Agent => "agent",
        StepKind::Command => "command",
    }
}

fn step_status_label(status: &StepStatus) -> &'static str {
    match status {
        StepStatus::Completed => "completed",
        StepStatus::Failed => "failed",
    }
}

fn slugify(value: &str) -> String {
    let mut slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();

    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }

    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "step".to_owned()
    } else {
        slug.to_owned()
    }
}

fn format_invocation(binary: &str, args: &[String]) -> String {
    std::iter::once(binary)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
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
            TaskStep::Agent { agent, task } => {
                serde_json::json!({ "agent": agent, "task": task })
            }
            TaskStep::Command { command } => serde_json::json!({ "command": command }),
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "agents": agents,
        "steps": steps,
        "commands": spec.commands,
    })
}
