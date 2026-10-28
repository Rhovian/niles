use std::process::{Command, ExitStatus, Stdio};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::{
    agents::{self, BriefDelivery},
    config::spec::load_project_config_from,
    skills,
    util::render_template,
    watch,
    workspace_manifest::WorkspaceManifest,
};

use super::brief::{ManagerSession, prepare_manager_session};

const SIGNAL_EXIT_LABEL: &str = "signal";

pub(super) fn launch_foreground_agent(
    workspace: &Utf8Path,
    manifest: &WorkspaceManifest,
) -> Result<()> {
    let agent = &manifest.binding(&manifest.entry)?.standard.primary;
    let mut invocation = foreground_invocation_for_project(workspace, agent)?;
    let primary = skills::resolve(&manifest.entry, workspace, &skills::global_dir()?)?;
    let prepared = prepare_manager_session(
        workspace,
        &invocation.spec,
        &manifest.entry,
        manifest.source.as_ref(),
        primary.primary_body()?,
    )?;
    invocation.pass_primary(&prepared.brief)?;
    if let Some(file) = &manifest.source {
        invocation.env.push((
            "NILES_SETUP".into(),
            format!("{}:{}", file.setup.origin, file.setup.name),
        ));
        eprintln!("setup: {file}");
    }
    eprintln!("skill: {primary}");
    prepared.write()?;
    let ManagerSession { meta, dir, .. } = prepared;
    if let Some(link) = &meta.session_link {
        invocation.args.extend(link.args());
    }
    let prompt = entry_prompt_args(invocation.brief, &dir);
    invocation.args.extend(prompt);

    // The watcher is held for exactly as long as the foreground agent runs, on the failing path
    // too: dropping it stops and joins the thread.
    let _watcher = watch::start(
        &dir,
        workspace,
        meta.lead_pane.as_deref(),
        invocation.spec.profile().composer,
    );

    let status = run_foreground_process(
        workspace,
        invocation.binary,
        &invocation.args,
        &invocation.env,
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
        .stderr(Stdio::inherit())
        .stdin(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to launch foreground agent `{binary}`"))
}

fn foreground_invocation_for_project(
    root: &Utf8Path,
    agent: &str,
) -> Result<agents::AgentInvocation> {
    let config = load_project_config_from(root)?;
    agents::invocation(
        agent,
        agents::InvocationDefaults::Foreground,
        &config.models,
    )
}

fn entry_prompt_args(delivery: BriefDelivery, dir: &Utf8Path) -> Vec<String> {
    const ENTRY_TURN_TEMPLATE: &str =
        "# Niles start session\nsession_dir: {dir}\n\nStart the Niles manager session.";
    let turn = render_template(ENTRY_TURN_TEMPLATE, &[("{dir}", dir.as_str())]);
    match delivery {
        BriefDelivery::Arg => vec![turn],
        BriefDelivery::Flag { value, .. } => vec![value.to_owned(), turn],
    }
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
    fn run_foreground_process_applies_cwd_and_env() {
        let root = temp_test_path("foreground-process");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let script = root.join("manager");
        let args_log = root.join("args.log");
        let pwd_log = root.join("pwd.log");
        let env_log = root.join("env.log");
        write_executable_script(
            &script,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\npwd > {}\nprintf '%s\\n' \"$NILES_FOREGROUND_ENV_TEST\" > {}\n",
                shell_quote(args_log.as_str()),
                shell_quote(pwd_log.as_str()),
                shell_quote(env_log.as_str()),
            ),
        );

        let args = ["--mode", "manager"].map(str::to_owned);
        let env = vec![(
            "NILES_FOREGROUND_ENV_TEST".to_owned(),
            "from-invocation".to_owned(),
        )];
        let status = run_foreground_process(&workspace, script.as_str(), &args, &env).unwrap();

        assert!(status.success());
        let args_body = fs::read_to_string(args_log).unwrap();
        assert_eq!(args_body, "--mode\nmanager\n");
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
            run_foreground_process(&root, script.as_str(), &[], &[])
                .unwrap()
                .code(),
            Some(42)
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn entry_prompt_args_seed_a_hermes_query() {
        let args = entry_prompt_args(
            agents::profile_for("hermes").unwrap().opening_turn,
            Utf8Path::new("/tmp/session"),
        );

        assert_eq!(args[0], "-q");
        assert!(args[1].starts_with("# Niles start session\nsession_dir: /tmp/session\n"));
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn entry_header_does_not_expand_replacement_values() {
        let prompt = entry_prompt_args(BriefDelivery::Arg, Utf8Path::new("/tmp/{dir}"));
        assert!(prompt[0].starts_with("# Niles start session\nsession_dir: /tmp/{dir}\n"));
    }
}
