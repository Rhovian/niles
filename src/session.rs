use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{self, BufRead, IsTerminal, Write},
    process::{Command, ExitStatus, Stdio},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    agents,
    config::spec::{PromptMode, load_project_config_from},
    process::exit_code_label,
    schema::{self, ArtifactKind},
    store, tmux,
    util::{current_dir_utf8, read_dir_utf8_paths, timestamp_id, write_json_pretty},
    wake,
    workspace_manifest::{self, WorkspaceManifest},
};

const MANAGER_BRIEF_TEMPLATE: &str = include_str!("templates/manager_brief.md");
const FOREGROUND_TMUX_SESSION: &str = "niles";
const STARTUP_PROMPT: &str = "Start the Niles manager session.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    pub workspace: Utf8PathBuf,
    pub brief: Utf8PathBuf,
}

pub fn run(manager: Option<String>) -> Result<()> {
    let workspace = current_dir_utf8()?;
    if !ensure_tmux_session(env::var_os("TMUX").as_deref(), &workspace)? {
        return Ok(());
    }

    let manifest = launch_prelude(&workspace, manager.as_deref())?;
    launch_foreground_agent(&workspace, &manifest)
}

fn launch_prelude(
    workspace: &Utf8Path,
    manager_override: Option<&str>,
) -> Result<WorkspaceManifest> {
    let worker_dir = workspace.join(".niles").join("worker");
    fs::create_dir_all(&worker_dir).with_context(|| format!("failed to create {worker_dir}"))?;

    let mut defaults = WorkspaceManifest::default();
    if let Some(manager) = manager_override {
        defaults.manager = manager.to_owned();
    }

    workspace_manifest::ensure_interactive(workspace, &defaults)
}

fn ensure_tmux_session(tmux: Option<&OsStr>, workspace: &Utf8Path) -> Result<bool> {
    let terminal = terminal_mode_from_stdio();
    let foreground_session_exists = if tmux_session_present(tmux) || !terminal.interactive() {
        false
    } else {
        tmux::has_session(FOREGROUND_TMUX_SESSION)
    };

    match tmux_launch_action(tmux, terminal, foreground_session_exists)? {
        TmuxLaunchAction::ContinueHere => Ok(true),
        TmuxLaunchAction::StartSession { session } => {
            let status = tmux::launch_foreground_session(&session, workspace, &current_argv()?)?;
            ensure_tmux_launch_succeeded(status)?;
            Ok(false)
        }
        TmuxLaunchAction::PromptExistingSession => {
            let stdin = io::stdin();
            let mut input = stdin.lock();
            let mut output = io::stdout();
            let action =
                prompt_existing_session_action(&mut input, &mut output, FOREGROUND_TMUX_SESSION)?;
            let status = match action {
                ExistingSessionAction::AttachExisting => {
                    tmux::attach_foreground_session(FOREGROUND_TMUX_SESSION)?
                }
                ExistingSessionAction::StartNamedSession(session) => {
                    tmux::launch_foreground_session(&session, workspace, &current_argv()?)?
                }
            };
            ensure_tmux_launch_succeeded(status)?;
            Ok(false)
        }
    }
}

fn ensure_tmux_launch_succeeded(status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        bail!("tmux foreground launch exited with {status}")
    }
}

fn current_argv() -> Result<Vec<OsString>> {
    let argv = env::args_os().collect::<Vec<_>>();
    if argv.is_empty() {
        bail!("failed to determine original argv for tmux foreground launch");
    }
    Ok(argv)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerminalMode {
    stdin: bool,
    stdout: bool,
}

impl TerminalMode {
    fn interactive(self) -> bool {
        self.stdin && self.stdout
    }
}

fn terminal_mode_from_stdio() -> TerminalMode {
    TerminalMode {
        stdin: io::stdin().is_terminal(),
        stdout: io::stdout().is_terminal(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TmuxLaunchAction {
    ContinueHere,
    StartSession { session: &'static str },
    PromptExistingSession,
}

fn tmux_launch_action(
    tmux: Option<&OsStr>,
    terminal: TerminalMode,
    foreground_session_exists: bool,
) -> Result<TmuxLaunchAction> {
    if tmux_session_present(tmux) {
        return Ok(TmuxLaunchAction::ContinueHere);
    }

    if !terminal.interactive() {
        bail!(
            "Niles must launch the foreground manager from an attached tmux session. Because stdin and stdout are not both TTYs, Niles will not auto-start tmux; start or attach tmux interactively and run `niles` again."
        );
    }

    if foreground_session_exists {
        Ok(TmuxLaunchAction::PromptExistingSession)
    } else {
        Ok(TmuxLaunchAction::StartSession {
            session: FOREGROUND_TMUX_SESSION,
        })
    }
}

fn tmux_session_present(tmux: Option<&OsStr>) -> bool {
    tmux.and_then(OsStr::to_str)
        .is_some_and(|value| !value.trim().is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExistingSessionAction {
    AttachExisting,
    StartNamedSession(String),
}

fn prompt_existing_session_action<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    existing_session: &str,
) -> Result<ExistingSessionAction> {
    writeln!(output, "tmux session `{existing_session}` already exists.")?;
    writeln!(output, "Choose how Niles should continue:")?;
    writeln!(output, "1. Attach to `{existing_session}`.")?;
    writeln!(
        output,
        "2. Start a new tmux session with another name and run this command."
    )?;

    loop {
        write!(output, "Selection [1/2]: ")?;
        output.flush()?;

        let mut line = String::new();
        let bytes = input
            .read_line(&mut line)
            .context("failed to read tmux session selection")?;
        if bytes == 0 {
            bail!("stdin closed before tmux session selection was read");
        }

        match line.trim().to_ascii_lowercase().as_str() {
            "1" | "a" | "attach" => return Ok(ExistingSessionAction::AttachExisting),
            "2" | "n" | "new" => {
                return prompt_new_session_name(input, output, existing_session);
            }
            _ => writeln!(output, "Please answer 1 or 2.")?,
        }
    }
}

fn prompt_new_session_name<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    existing_session: &str,
) -> Result<ExistingSessionAction> {
    loop {
        write!(output, "New tmux session name: ")?;
        output.flush()?;

        let mut line = String::new();
        let bytes = input
            .read_line(&mut line)
            .context("failed to read new tmux session name")?;
        if bytes == 0 {
            bail!("stdin closed before new tmux session name was read");
        }

        let session = line.trim();
        if session.is_empty() {
            writeln!(output, "New tmux session name cannot be empty.")?;
        } else if session == existing_session {
            writeln!(
                output,
                "New tmux session name must differ from `{existing_session}`."
            )?;
        } else {
            return Ok(ExistingSessionAction::StartNamedSession(session.to_owned()));
        }
    }
}

fn launch_foreground_agent(workspace: &Utf8Path, manifest: &WorkspaceManifest) -> Result<()> {
    let agent = &manifest.manager;
    let invocation = foreground_invocation_for_project(workspace, agent)?;
    let binary = invocation.binary;
    let mut args = invocation.args;
    let family = invocation.spec.family().to_owned();
    let meta = write_manager_session(workspace, &invocation.spec, manifest)?;
    let brief = fs::read_to_string(&meta.brief)
        .with_context(|| format!("failed to read manager brief {}", meta.brief))?;
    let prompt = manager_prompt_io(&family, invocation.prompt, brief);
    args.extend(prompt.args);

    let status = run_foreground_process(workspace, &binary, &args, prompt.stdin.as_deref())?;

    if status.success() {
        Ok(())
    } else {
        bail!(
            "foreground agent `{agent}` exited with {}",
            exit_code_label(status.code())
        )
    }
}

fn run_foreground_process(
    workspace: &Utf8Path,
    binary: &str,
    args: &[String],
    stdin: Option<&str>,
) -> Result<ExitStatus> {
    let mut command = Command::new(binary);
    command
        .current_dir(workspace)
        .args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    match stdin {
        Some(stdin) => {
            let mut child = command
                .stdin(Stdio::piped())
                .spawn()
                .with_context(|| format!("failed to launch foreground agent `{binary}`"))?;
            let mut child_stdin = child
                .stdin
                .take()
                .context("failed to open foreground agent stdin pipe")?;
            child_stdin.write_all(stdin.as_bytes()).with_context(|| {
                format!("failed to write foreground agent stdin for `{binary}`")
            })?;
            drop(child_stdin);
            child
                .wait()
                .with_context(|| format!("failed to wait for foreground agent `{binary}`"))
        }
        None => command
            .stdin(Stdio::inherit())
            .status()
            .with_context(|| format!("failed to launch foreground agent `{binary}`")),
    }
}

fn foreground_invocation_for_project(
    root: &Utf8Path,
    agent: &str,
) -> Result<agents::AgentInvocation> {
    let config = load_project_config_from(root)?;
    let agent_config = agents::config_for(&config.agents, agent)?;
    agents::foreground_invocation(agent, agent_config)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForegroundPrompt {
    args: Vec<String>,
    stdin: Option<String>,
}

fn manager_prompt_io(agent: &str, prompt: PromptMode, brief: String) -> ForegroundPrompt {
    match prompt {
        PromptMode::Arg => ForegroundPrompt {
            args: manager_prompt_args(agent, brief),
            stdin: None,
        },
        PromptMode::Stdin => ForegroundPrompt {
            args: Vec::new(),
            stdin: Some(manager_stdin_prompt(brief)),
        },
    }
}

fn manager_prompt_args(agent: &str, brief: String) -> Vec<String> {
    agents::manager_prompt_args(agent, brief, STARTUP_PROMPT.to_owned())
}

fn manager_stdin_prompt(brief: String) -> String {
    format!("{brief}\n\n{STARTUP_PROMPT}")
}

fn write_manager_session(
    workspace: &Utf8Path,
    agent: &agents::AgentSpec,
    manifest: &WorkspaceManifest,
) -> Result<SessionMeta> {
    let now = Utc::now();
    let id = timestamp_id(&now);
    let dir = workspace.join(".niles").join("sessions").join(&id);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;
    let path = dir.join("manager.md");
    let startup_context = startup_context(workspace)?;
    let body = render_manager_brief(agent, workspace, &dir, manifest, &startup_context);
    fs::write(&path, body).with_context(|| format!("failed to write {path}"))?;
    let meta = SessionMeta {
        id: id.clone(),
        agent: agent.canonical(),
        agent_family: agent.tier().map(|tier| tier.family),
        model: agent.model().map(str::to_owned),
        effort: agent.effort().map(str::to_owned),
        workspace: workspace.to_path_buf(),
        brief: path,
    };
    let meta_path = session_meta_path(workspace, &id);
    write_json_pretty(&meta_path, &meta)?;
    fs::write(latest_session_path(workspace), &id)
        .context("failed to write latest session pointer")?;
    Ok(meta)
}

fn render_manager_brief(
    agent: &agents::AgentSpec,
    workspace: &Utf8Path,
    dir: &Utf8Path,
    manifest: &WorkspaceManifest,
    startup_context: &str,
) -> String {
    let manifest_path = workspace_manifest::manifest_path(workspace);
    let flow = workspace_manifest::flow_summary(&manifest.flow);
    MANAGER_BRIEF_TEMPLATE
        .replace("{workspace}", workspace.as_str())
        .replace("{agent}", &agent.canonical())
        .replace("{dir}", dir.as_str())
        .replace("{manifest}", manifest_path.as_str())
        .replace("{flow}", &flow)
        .replace("{startup_context}", startup_context)
        .replace(
            "{worker_wake_examples}",
            &wake::manager_worker_contract_examples("<status-file>"),
        )
}

fn session_meta_path(workspace: &Utf8Path, id: &str) -> Utf8PathBuf {
    workspace
        .join(".niles")
        .join("sessions")
        .join(id)
        .join("session.json")
}

fn latest_session_path(workspace: &Utf8Path) -> Utf8PathBuf {
    workspace.join(".niles").join("sessions").join("latest")
}

fn startup_context(workspace: &Utf8Path) -> Result<String> {
    let lines = [latest_run_context(workspace)?, worker_context(workspace)?];
    Ok(lines.join("\n"))
}

fn latest_run_context(workspace: &Utf8Path) -> Result<String> {
    let Some(run_dir) = store::resolve_latest_run_dir_from(workspace)? else {
        return Ok("latest_run: none".to_owned());
    };

    let state_path = run_dir.join("state.json");
    let state = schema::read_json::<crate::state::RunState>(&state_path, ArtifactKind::RunState)?;
    Ok(format!(
        "latest_run: id={} status={} goal={:?}",
        state.id, state.status, state.goal
    ))
}

fn worker_context(workspace: &Utf8Path) -> Result<String> {
    let worker_dir = workspace.join(".niles").join("worker");
    let mut ids = read_dir_utf8_paths(&worker_dir)?
        .into_iter()
        .filter(|path| path.extension() == Some("json"))
        .filter_map(|path| {
            path.file_stem()
                .map(|stem| stem.to_owned())
                .filter(|stem| !stem.is_empty())
        })
        .collect::<Vec<_>>();
    ids.sort();
    if ids.is_empty() {
        Ok("worker: none".to_owned())
    } else {
        Ok(format!("worker: {}", ids.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{
        os::unix::fs::PermissionsExt,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn tmux_session_present_accepts_nonempty_env() {
        assert!(tmux_session_present(Some(OsStr::new(
            "/tmp/tmux-501/default,1,0"
        ))));
    }

    #[test]
    fn tmux_session_present_rejects_missing_or_empty_env() {
        assert!(!tmux_session_present(None));
        assert!(!tmux_session_present(Some(OsStr::new(""))));
        assert!(!tmux_session_present(Some(OsStr::new("   "))));
    }

    #[test]
    fn tmux_launch_action_continues_inside_tmux_even_without_tty() {
        let action = tmux_launch_action(
            Some(OsStr::new("/tmp/tmux-501/default,1,0")),
            TerminalMode {
                stdin: false,
                stdout: false,
            },
            true,
        )
        .unwrap();

        assert_eq!(action, TmuxLaunchAction::ContinueHere);
    }

    #[test]
    fn tmux_launch_action_starts_session_for_interactive_no_tmux() {
        let action = tmux_launch_action(
            None,
            TerminalMode {
                stdin: true,
                stdout: true,
            },
            false,
        )
        .unwrap();

        assert_eq!(
            action,
            TmuxLaunchAction::StartSession {
                session: FOREGROUND_TMUX_SESSION,
            }
        );
    }

    #[test]
    fn tmux_launch_action_prompts_for_interactive_existing_session() {
        let action = tmux_launch_action(
            None,
            TerminalMode {
                stdin: true,
                stdout: true,
            },
            true,
        )
        .unwrap();

        assert_eq!(action, TmuxLaunchAction::PromptExistingSession);
    }

    #[test]
    fn tmux_launch_action_errors_for_non_tty_no_tmux() {
        let err = tmux_launch_action(
            None,
            TerminalMode {
                stdin: true,
                stdout: false,
            },
            true,
        )
        .unwrap_err();

        assert!(err.to_string().contains("not both TTYs"));
        assert!(err.to_string().contains("will not auto-start tmux"));
    }

    #[test]
    fn existing_session_prompt_can_attach_existing_session() {
        let mut input = io::Cursor::new(b"attach\n".to_vec());
        let mut output = Vec::new();

        let action =
            prompt_existing_session_action(&mut input, &mut output, FOREGROUND_TMUX_SESSION)
                .unwrap();

        assert_eq!(action, ExistingSessionAction::AttachExisting);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("tmux session `niles` already exists."));
        assert!(output.contains("Selection [1/2]: "));
        assert!(!output.contains("new-session -A"));
    }

    #[test]
    fn existing_session_prompt_reads_different_new_session_name() {
        let mut input = io::Cursor::new(b"2\n\nniles\nniles-2\n".to_vec());
        let mut output = Vec::new();

        let action =
            prompt_existing_session_action(&mut input, &mut output, FOREGROUND_TMUX_SESSION)
                .unwrap();

        assert_eq!(
            action,
            ExistingSessionAction::StartNamedSession("niles-2".to_owned())
        );
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("New tmux session name cannot be empty."));
        assert!(output.contains("must differ from `niles`"));
    }

    #[test]
    fn foreground_invocation_for_project_preserves_builtin_manager_defaults() {
        let root = temp_test_path("builtin-manager");

        let invocation = foreground_invocation_for_project(&root, "claude:opus:max").unwrap();

        assert_eq!(invocation.binary, "claude");
        assert_eq!(
            invocation.args,
            ["--model", "opus", "--effort", "max"].map(str::to_owned)
        );
    }

    #[test]
    fn foreground_invocation_for_project_uses_configured_custom_manager() {
        let root = temp_test_path("custom-manager");
        fs::create_dir_all(&root).unwrap();
        let binary = root.join("custom-manager");
        fs::write(
            root.join("niles.yaml"),
            format!(
                r#"
agents:
  gemini:
    binary: {}
    args:
      - --mode
      - manager
"#,
                binary
            ),
        )
        .unwrap();

        let invocation = foreground_invocation_for_project(&root, "gemini").unwrap();

        assert_eq!(invocation.binary, binary.as_str());
        assert_eq!(invocation.args, ["--mode", "manager"].map(str::to_owned));
        assert_eq!(invocation.spec.family(), "gemini");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configured_custom_manager_stdin_prompt_keeps_prompt_out_of_args() {
        let root = temp_test_path("custom-manager-stdin");
        fs::create_dir_all(&root).unwrap();
        let binary = root.join("custom-manager");
        fs::write(
            root.join("niles.yaml"),
            format!(
                r#"
agents:
  gemini:
    binary: {}
    args:
      - --mode
      - manager
    prompt: stdin
"#,
                binary
            ),
        )
        .unwrap();

        let invocation = foreground_invocation_for_project(&root, "gemini").unwrap();
        let prompt = manager_prompt_io(
            invocation.spec.family(),
            invocation.prompt,
            "brief body".to_owned(),
        );
        let mut args = invocation.args;
        args.extend(prompt.args);

        assert!(matches!(invocation.prompt, PromptMode::Stdin));
        assert_eq!(args, ["--mode", "manager"].map(str::to_owned));
        assert!(args.iter().all(|arg| !arg.contains("brief body")));
        assert_eq!(
            prompt.stdin.as_deref(),
            Some("brief body\n\nStart the Niles manager session.")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn run_foreground_process_writes_stdin_without_prompt_args() {
        let root = temp_test_path("foreground-stdin-process");
        fs::create_dir_all(&root).unwrap();
        let script = root.join("manager");
        let args_log = root.join("args.log");
        let stdin_log = root.join("stdin.log");
        write_executable_script(
            &script,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat > '{}'\n",
                args_log, stdin_log
            ),
        );

        let args = ["--mode", "manager"].map(str::to_owned);
        let prompt = "brief body\n\nStart the Niles manager session.";
        let status = run_foreground_process(&root, script.as_str(), &args, Some(prompt)).unwrap();

        assert!(status.success());
        let args_body = fs::read_to_string(args_log).unwrap();
        assert_eq!(args_body, "--mode\nmanager\n");
        assert!(!args_body.contains("brief body"));
        assert_eq!(fs::read_to_string(stdin_log).unwrap(), prompt);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn run_foreground_process_uses_explicit_workspace_cwd() {
        let root = temp_test_path("foreground-explicit-cwd");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let script = root.join("manager");
        let pwd_log = root.join("pwd.log");
        write_executable_script(
            &script,
            &format!("#!/bin/sh\npwd > {}\n", shell_quote(&pwd_log)),
        );

        let args = Vec::new();
        let status = run_foreground_process(&workspace, script.as_str(), &args, None).unwrap();

        assert!(status.success());
        let expected = Utf8PathBuf::from_path_buf(fs::canonicalize(&workspace).unwrap()).unwrap();
        assert_eq!(
            fs::read_to_string(pwd_log).unwrap().trim_end(),
            expected.as_str()
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn run_foreground_process_returns_nonzero_child_status() {
        let root = temp_test_path("foreground-nonzero-status");
        fs::create_dir_all(&root).unwrap();
        let script = root.join("manager");
        write_executable_script(&script, "#!/bin/sh\nexit 42\n");

        let args = Vec::new();
        let status = run_foreground_process(&root, script.as_str(), &args, None).unwrap();

        assert_eq!(status.code(), Some(42));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manager_prompt_args_pass_brief_as_claude_system_prompt() {
        let args = manager_prompt_args("claude", "brief body".to_owned());

        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "--append-system-prompt");
        assert_eq!(args[1], "brief body");
        assert_eq!(args[2], "Start the Niles manager session.");
    }

    #[test]
    fn manager_prompt_io_preserves_claude_arg_mode_system_prompt() {
        let prompt = manager_prompt_io("claude", PromptMode::Arg, "brief body".to_owned());

        assert_eq!(prompt.stdin, None);
        assert_eq!(prompt.args.len(), 3);
        assert_eq!(prompt.args[0], "--append-system-prompt");
        assert_eq!(prompt.args[1], "brief body");
        assert_eq!(prompt.args[2], "Start the Niles manager session.");
    }

    #[test]
    fn manager_brief_omits_removed_manifest_command() {
        assert!(MANAGER_BRIEF_TEMPLATE.contains("niles spawn <id>"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("niles run"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("manifest: {manifest}"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("flow: {flow}"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("source of truth"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("standard worker-verification-reviewer loop"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("CONSENSUS OR ESCALATE"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("Do not generate a task YAML file"));
        assert!(!MANAGER_BRIEF_TEMPLATE.contains("niles manifest"));
    }

    #[test]
    fn cost_discipline_guidance() {
        assert_eq!(MANAGER_BRIEF_TEMPLATE.matches("## Cost Discipline").count(), 1);
        assert!(MANAGER_BRIEF_TEMPLATE.contains("Tier reviewers by surface risk"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("Scope re-reviews to the fix delta"));
        assert!(MANAGER_BRIEF_TEMPLATE.contains("Keep manager context lean"));
    }

    #[test]
    fn manager_brief_render_includes_manifest_flow() {
        let workspace = temp_test_path("brief-render");
        let dir = workspace.join(".niles/sessions/test-session");
        let agent = agents::parse_spec("codex:gpt-5.5:xhigh").unwrap();
        let manifest = WorkspaceManifest {
            manager: "codex:gpt-5.5:xhigh".to_owned(),
            planner: "planbot".to_owned(),
            worker: "codebot".to_owned(),
            reviewer: "reviewbot".to_owned(),
            validation_command: "check".to_owned(),
            flow: vec![
                workspace_manifest::WorkspaceFlowRole::Reviewer,
                workspace_manifest::WorkspaceFlowRole::Validation,
            ],
        };

        let body = render_manager_brief(&agent, &workspace, &dir, &manifest, "latest_run: none");

        assert!(body.contains(&format!(
            "manifest: {}",
            workspace_manifest::manifest_path(&workspace)
        )));
        assert!(body.contains("flow: reviewer -> validation"));
        assert!(body.contains("manager_agent: codex:gpt-5.5:xhigh"));
        assert!(body.contains("latest_run: none"));
        assert!(!body.contains("{manifest}"));
        assert!(!body.contains("{flow}"));
    }

    #[test]
    fn latest_run_context_reads_typed_state_fields() {
        let root = temp_test_path("latest-run-context-valid");
        let run_dir = root.join(".niles/runs/run-1");
        fs::create_dir_all(&run_dir).unwrap();
        let now = Utc::now();
        let state = crate::state::RunState {
            id: "run-1".to_owned(),
            goal: "Ship typed context".to_owned(),
            workspace: Some(root.clone()),
            config_root: None,
            task_file: None,
            created_at: now,
            updated_at: now,
            status: crate::state::RunStatus::Running,
            steps: Vec::new(),
        };
        schema::write_json(&run_dir.join("state.json"), &state).unwrap();

        let context = latest_run_context(&root).unwrap();

        assert_eq!(
            context,
            r#"latest_run: id=run-1 status=running goal="Ship typed context""#
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn latest_run_context_errors_when_state_missing_required_field() {
        let root = temp_test_path("latest-run-context-invalid");
        let run_dir = root.join(".niles/runs/run-1");
        fs::create_dir_all(&run_dir).unwrap();
        let now = Utc::now().to_rfc3339();
        let state = serde_json::json!({
            "id": "run-1",
            "created_at": now,
            "updated_at": now,
            "status": "running",
            "steps": []
        });
        schema::write_json(&run_dir.join("state.json"), &state).unwrap();

        let err = latest_run_context(&root).unwrap_err();
        let message = format!("{err:#}");

        assert!(message.contains("missing field `goal`"));
        assert!(!message.contains("unknown"));

        fs::remove_dir_all(root).unwrap();
    }

    fn temp_test_path(label: &str) -> Utf8PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
            "niles-session-{label}-{}-{nanos}",
            std::process::id()
        )))
        .unwrap()
    }

    fn write_executable_script(path: &Utf8Path, body: &str) {
        fs::write(path, body).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    fn shell_quote(path: &Utf8Path) -> String {
        format!("'{}'", path.as_str().replace('\'', "'\\''"))
    }
}
