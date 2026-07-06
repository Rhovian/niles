use std::fs;

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::{
    agent_window::{self, AgentWindowPrompt},
    tmux,
    workspace_manifest::WorkspaceManifest,
};

use super::{
    SessionMeta,
    brief::{read_latest_session, write_manager_session, write_session_meta},
    foreground::{ManagerCommand, foreground_invocation_for_project, prepare_manager_command},
};

pub(super) const MANAGER_WINDOW_NAME: &str = "niles-manager";

pub(super) fn ensure_manager_window(
    workspace: &Utf8Path,
    manifest: &WorkspaceManifest,
) -> Result<SessionMeta> {
    let session = tmux::current_or_named_session("niles")?;
    let expected_target = format!("{session}:{MANAGER_WINDOW_NAME}");
    if tmux::target_exists(&expected_target)? {
        return reuse_existing_manager_window(workspace, &expected_target);
    }

    spawn_manager_window(workspace, manifest)
}

fn reuse_existing_manager_window(
    workspace: &Utf8Path,
    expected_target: &str,
) -> Result<SessionMeta> {
    let Some(meta) = read_latest_session(workspace)? else {
        bail!(
            "tmux window {expected_target} already exists, but latest manager session metadata is missing; close the stale window or use `niles -d` for the legacy foreground manager"
        );
    };

    match meta.window.as_deref() {
        Some(window) if window == expected_target => Ok(meta),
        Some(window) => bail!(
            "tmux window {expected_target} already exists, but latest manager session metadata points at {window}; close the stale window or use `niles -d` for the legacy foreground manager"
        ),
        None => bail!(
            "tmux window {expected_target} already exists, but latest manager session metadata has no manager window; close the stale window or use `niles -d` for the legacy foreground manager"
        ),
    }
}

fn spawn_manager_window(workspace: &Utf8Path, manifest: &WorkspaceManifest) -> Result<SessionMeta> {
    let invocation = foreground_invocation_for_project(workspace, &manifest.manager)?;
    let mut meta = write_manager_session(workspace, &invocation.spec, manifest)?;
    let brief = fs::read_to_string(&meta.brief)
        .with_context(|| format!("failed to read manager brief {}", meta.brief))?;
    let command = prepare_manager_window_command(invocation, brief)?;
    let launch_path = meta
        .brief
        .parent()
        .context("manager brief path has no session directory")?
        .join("launch.sh");
    let target = agent_window::spawn_prepared_agent_window(
        MANAGER_WINDOW_NAME,
        workspace,
        &command.invocation,
        &launch_path,
        AgentWindowPrompt::Prepared {
            stdin: command.stdin.as_deref(),
        },
    )?;

    meta.window = Some(target);
    meta.launch = Some(launch_path);
    write_session_meta(workspace, &meta)?;
    Ok(meta)
}

fn prepare_manager_window_command(
    invocation: crate::agents::AgentInvocation,
    brief: String,
) -> Result<ManagerCommand> {
    prepare_manager_command(invocation, brief)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents;

    #[test]
    fn manager_window_prompt_matches_foreground_for_claude() {
        assert_prompt_parity("claude:opus:max", "brief body");
    }

    #[test]
    fn manager_window_prompt_matches_foreground_for_combined_arg_agent() {
        let brief = "brief body";
        let window = assert_prompt_parity("codex", brief);

        assert_eq!(window.stdin, None);
        assert_eq!(
            window.invocation.args,
            vec!["brief body\n\nStart the Niles manager session.".to_owned()]
        );
    }

    fn assert_prompt_parity(agent: &str, brief: &str) -> ManagerCommand {
        let invocation = agents::foreground_invocation(agent, None).unwrap();
        let foreground = prepare_manager_command(invocation.clone(), brief.to_owned()).unwrap();
        let window = prepare_manager_window_command(invocation, brief.to_owned()).unwrap();

        assert_eq!(window.invocation.args, foreground.invocation.args);
        assert_eq!(window.stdin, foreground.stdin);
        window
    }
}
