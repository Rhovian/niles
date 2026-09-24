use std::{
    fs,
    io::Write,
    process::{Command, ExitStatus, Stdio},
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::{
    agents::{self, BriefDelivery},
    config::spec::load_project_config_from,
    watch,
    workspace_manifest::WorkspaceManifest,
};

use super::{SessionMeta, brief::write_manager_session};

const STARTUP_PROMPT: &str = "Start the Niles manager session.";
const SIGNAL_EXIT_LABEL: &str = "signal";

pub(super) fn launch_foreground_agent(
    workspace: &Utf8Path,
    manifest: &WorkspaceManifest,
) -> Result<()> {
    let agent = &manifest.lead;
    let invocation = foreground_invocation_for_project(workspace, agent)?;
    let meta: SessionMeta = write_manager_session(workspace, &invocation.spec)?;
    let brief = fs::read_to_string(&meta.brief)
        .with_context(|| format!("failed to read manager brief {}", meta.brief))?;
    let session_dir = meta
        .brief
        .parent()
        .with_context(|| format!("manager brief has no session directory: {}", meta.brief))?;
    let command = prepare_manager_command(invocation, brief);

    // The watcher is held for exactly as long as the foreground agent runs, on the failing path
    // too: dropping it stops and joins the thread.
    let _watcher = watch::start(session_dir, workspace, meta.lead_pane.as_deref());

    let status = run_foreground_process(
        workspace,
        &command.invocation.binary,
        &command.invocation.args,
        &command.invocation.env,
        command.stdin.as_deref(),
    )?;

    if status.success() {
        Ok(())
    } else {
        bail!(
            "foreground agent `{agent}` exited with {}",
            exit_code_label(status.code())
        )
    }
}

#[derive(Debug, Clone)]
pub(super) struct ManagerCommand {
    pub(super) invocation: agents::AgentInvocation,
    pub(super) stdin: Option<String>,
}

pub(super) fn prepare_manager_command(
    mut invocation: agents::AgentInvocation,
    brief: String,
) -> ManagerCommand {
    let prompt = manager_prompt_io(invocation.brief, brief);
    invocation.args.extend(prompt.args);
    ManagerCommand {
        invocation,
        stdin: prompt.stdin,
    }
}

fn run_foreground_process(
    workspace: &Utf8Path,
    binary: &str,
    args: &[String],
    env: &[(String, String)],
    stdin: Option<&str>,
) -> Result<ExitStatus> {
    let mut command = Command::new(binary);
    command
        .current_dir(workspace)
        .args(args)
        .envs(
            env.iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        )
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

pub(super) fn foreground_invocation_for_project(
    root: &Utf8Path,
    agent: &str,
) -> Result<agents::AgentInvocation> {
    let config = load_project_config_from(root)?;
    let agent_config = agents::config_for(&config.agents, agent)?;
    agents::foreground_invocation(agent, agent_config)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ForegroundPrompt {
    args: Vec<String>,
    stdin: Option<String>,
}

/// Renders the lead's half of the one brief-delivery decision: the same dial the worker's launch
/// script reads, spelled as argv for a process that inherits this pane.
///
/// The lead's brief is on disk — `write_manager_session` puts it at
/// `.niles/sessions/<id>/lead.md` — but that file holds the brief alone, and the lead's opening
/// turn is the brief *plus* the startup line. So a family that takes a worker's brief by path
/// still takes the lead's by value: it is the turn that has no file, not the brief.
pub(super) fn manager_prompt_io(delivery: BriefDelivery, brief: String) -> ForegroundPrompt {
    match delivery {
        BriefDelivery::Arg => ForegroundPrompt {
            args: vec![opening_turn(&brief)],
            stdin: None,
        },
        BriefDelivery::Stdin => ForegroundPrompt {
            args: Vec::new(),
            stdin: Some(opening_turn(&brief)),
        },
        BriefDelivery::Flag { value, .. } => ForegroundPrompt {
            args: vec![value.to_owned(), opening_turn(&brief)],
            stdin: None,
        },
        BriefDelivery::SystemPrompt(flag) => ForegroundPrompt {
            args: vec![flag.to_owned(), brief, STARTUP_PROMPT.to_owned()],
            stdin: None,
        },
    }
}

fn opening_turn(brief: &str) -> String {
    format!("{brief}\n\n{STARTUP_PROMPT}")
}

fn exit_code_label(code: Option<i32>) -> String {
    code.map(|code| code.to_string())
        .unwrap_or_else(|| SIGNAL_EXIT_LABEL.to_owned())
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{shell_quote, temp_test_path, write_executable_script};
    use super::*;

    use camino::Utf8PathBuf;

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
        let prompt = manager_prompt_io(invocation.brief, "brief body".to_owned());
        let mut args = invocation.args;
        args.extend(prompt.args);

        assert!(matches!(invocation.brief, BriefDelivery::Stdin));
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
        let env = Vec::new();
        let status =
            run_foreground_process(&root, script.as_str(), &args, &env, Some(prompt)).unwrap();

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
        let env = Vec::new();
        let status =
            run_foreground_process(&workspace, script.as_str(), &args, &env, None).unwrap();

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
        let env = Vec::new();
        let status = run_foreground_process(&root, script.as_str(), &args, &env, None).unwrap();

        assert_eq!(status.code(), Some(42));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn run_foreground_process_applies_invocation_env() {
        let root = temp_test_path("foreground-env");
        fs::create_dir_all(&root).unwrap();
        let script = root.join("manager");
        let env_log = root.join("env.log");
        write_executable_script(
            &script,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$NILES_FOREGROUND_ENV_TEST\" > {}\n",
                shell_quote(&env_log)
            ),
        );

        let args = Vec::new();
        let env = vec![(
            "NILES_FOREGROUND_ENV_TEST".to_owned(),
            "from-invocation".to_owned(),
        )];
        let status = run_foreground_process(&root, script.as_str(), &args, &env, None).unwrap();

        assert!(status.success());
        assert_eq!(
            fs::read_to_string(env_log).unwrap().trim_end(),
            "from-invocation"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manager_prompt_args_seed_a_hermes_query() {
        let invocation =
            foreground_invocation_for_project(&temp_test_path("hermes-lead"), "hermes").unwrap();

        let args = manager_prompt_io(invocation.brief, "brief body".to_owned()).args;

        assert_eq!(args[0], "-q");
        assert_eq!(args[1], format!("brief body\n\n{STARTUP_PROMPT}"));
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn manager_prompt_args_pass_brief_as_claude_system_prompt() {
        let invocation =
            foreground_invocation_for_project(&temp_test_path("claude-lead"), "claude").unwrap();

        let args = manager_prompt_io(invocation.brief, "brief body".to_owned()).args;

        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "--append-system-prompt");
        assert_eq!(args[1], "brief body");
        assert_eq!(args[2], "Start the Niles manager session.");
    }
}
