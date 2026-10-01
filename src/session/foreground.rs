use std::{
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

use super::brief::{ManagerSession, write_manager_session};

const STARTUP_PROMPT: &str = "Start the Niles manager session.";
const SIGNAL_EXIT_LABEL: &str = "signal";

pub(super) fn launch_foreground_agent(
    workspace: &Utf8Path,
    manifest: &WorkspaceManifest,
) -> Result<()> {
    let agent = &manifest.lead;
    let mut invocation = foreground_invocation_for_project(workspace, agent)?;
    let ManagerSession { meta, brief, dir } =
        write_manager_session(workspace, &invocation.spec, manifest)?;
    if let Some(link) = &meta.session_link {
        invocation.args.extend(link.args());
    }
    let prompt = manager_prompt_io(invocation.brief, brief);
    invocation.args.extend(prompt.args);

    // The watcher is held for exactly as long as the foreground agent runs, on the failing path
    // too: dropping it stops and joins the thread.
    let _watcher = watch::start(
        &dir,
        workspace,
        meta.lead_pane.as_deref(),
        agents::profile_for(invocation.spec.family()).and_then(|profile| profile.composer),
    );

    let status = run_foreground_process(
        workspace,
        &invocation.binary,
        &invocation.args,
        &invocation.env,
        prompt.stdin.as_deref(),
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

fn foreground_invocation_for_project(
    root: &Utf8Path,
    agent: &str,
) -> Result<agents::AgentInvocation> {
    let config = load_project_config_from(root)?;
    let agent_config = agents::config_for(&config.agents, agent, &config.models)?;
    agents::invocation(
        agent,
        agent_config,
        agents::InvocationDefaults::Foreground,
        &config.models,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForegroundPrompt {
    args: Vec<String>,
    stdin: Option<String>,
}

/// The lead's opening turn includes its brief and startup line, so deliveries by path use their
/// value flag: no single file contains the complete turn.
fn manager_prompt_io(delivery: BriefDelivery, brief: String) -> ForegroundPrompt {
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
    use super::super::test_support::write_executable_script;
    use super::*;
    use crate::{agent_window::shell_quote, test_support::temp_test_path};

    use camino::Utf8PathBuf;
    use std::fs;

    #[test]
    fn foreground_invocation_accepts_a_project_model_override() {
        let root = temp_test_path("foreground-model-override");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("niles.yaml"),
            "models: { codex: { gpt-5.7: { efforts: [xhigh] } } }",
        )
        .unwrap();

        let invocation = foreground_invocation_for_project(&root, "codex:gpt-5.7:xhigh").unwrap();

        assert!(
            invocation
                .args
                .windows(2)
                .any(|args| args == ["--model", "gpt-5.7"])
        );
        assert!(
            invocation
                .args
                .contains(&"model_reasoning_effort=\"xhigh\"".to_owned())
        );
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
    fn run_foreground_process_applies_stdin_cwd_and_env() {
        let root = temp_test_path("foreground-process");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let script = root.join("manager");
        let args_log = root.join("args.log");
        let stdin_log = root.join("stdin.log");
        let pwd_log = root.join("pwd.log");
        let env_log = root.join("env.log");
        write_executable_script(
            &script,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\ncat > {}\npwd > {}\nprintf '%s\\n' \"$NILES_FOREGROUND_ENV_TEST\" > {}\n",
                shell_quote(args_log.as_str()),
                shell_quote(stdin_log.as_str()),
                shell_quote(pwd_log.as_str()),
                shell_quote(env_log.as_str()),
            ),
        );

        let args = ["--mode", "manager"].map(str::to_owned);
        let prompt = "brief body\n\nStart the Niles manager session.";
        let env = vec![(
            "NILES_FOREGROUND_ENV_TEST".to_owned(),
            "from-invocation".to_owned(),
        )];
        let status =
            run_foreground_process(&workspace, script.as_str(), &args, &env, Some(prompt)).unwrap();

        assert!(status.success());
        let args_body = fs::read_to_string(args_log).unwrap();
        assert_eq!(args_body, "--mode\nmanager\n");
        assert!(!args_body.contains("brief body"));
        assert_eq!(fs::read_to_string(stdin_log).unwrap(), prompt);
        let expected = Utf8PathBuf::from_path_buf(fs::canonicalize(&workspace).unwrap()).unwrap();
        assert_eq!(
            fs::read_to_string(pwd_log).unwrap().trim_end(),
            expected.as_str()
        );
        assert_eq!(
            fs::read_to_string(env_log).unwrap().trim_end(),
            "from-invocation"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn run_foreground_process_returns_nonzero_child_status() {
        let root = temp_test_path("foreground-nonzero-status");
        fs::create_dir_all(&root).unwrap();
        let script = root.join("manager");
        write_executable_script(&script, "#!/bin/sh\nexit 42\n");

        assert_eq!(
            run_foreground_process(&root, script.as_str(), &[], &[], None)
                .unwrap()
                .code(),
            Some(42)
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manager_prompt_args_seed_a_hermes_query() {
        let args = manager_prompt_io(
            agents::profile_for("hermes").unwrap().lead_brief,
            "brief body".to_owned(),
        )
        .args;

        assert_eq!(args[0], "-q");
        assert_eq!(args[1], format!("brief body\n\n{STARTUP_PROMPT}"));
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn manager_prompt_args_pass_brief_as_claude_system_prompt() {
        let args = manager_prompt_io(
            agents::profile_for("claude").unwrap().lead_brief,
            "brief body".to_owned(),
        )
        .args;

        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "--append-system-prompt");
        assert_eq!(args[1], "brief body");
        assert_eq!(args[2], "Start the Niles manager session.");
    }
}
