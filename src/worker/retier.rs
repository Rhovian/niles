use super::{
    meta::{read_meta, write_meta},
    worker_dir,
};
use crate::{agents, config, tmux, util, workspace_manifest};
use anyhow::{Context, Result, bail, ensure};
use std::time::{Duration, Instant};

const CONFIRM_TIMEOUT: Duration = Duration::from_secs(20);
const CONFIRM_INTERVAL: Duration = Duration::from_millis(50);

pub fn retier(id: String, agent: String) -> Result<()> {
    let dir = worker_dir(&id)?;
    let mut meta = read_meta(&dir)?;
    let models = config::spec::load_project_config_from(&meta.project)?.models;
    let current = agents::AgentSpec::parse(&meta.agent, &models)?;
    let target = agents::AgentSpec::parse(&agent, &models)?;
    ensure!(current.family() == target.family(), "cannot change family");
    agents::validate_model(&target, &models)?;
    let model = target.model().context("expected family:model:effort")?;
    let effort = target.effort().context("expected family:model:effort")?;
    let steps = agents::switch_steps(target.family(), model, effort)?;
    let manifest =
        workspace_manifest::load(&meta.project)?.context("retier requires a workspace manifest")?;
    ensure!(
        meta.role.binding(&manifest).allows(&target, &models)?,
        "agent {agent} is not allowed for role '{}' in the manifest",
        meta.role.as_str()
    );
    let target_window = models
        .entry(target.family(), model, Some(effort))?
        .context_window
        .get();
    ensure!(
        matches!(super::resolve::window_state(&meta), tmux::TargetState::Live),
        "worker window not live"
    );
    let worker = super::worker_snapshot(&util::current_dir_utf8()?)?
        .into_iter()
        .find(|worker| worker.id == id)
        .context("worker disappeared")?;
    let session = super::worker_session(&worker, chrono::Utc::now())?;
    ensure!(
        session.state == crate::telemetry::SessionState::Waiting,
        "worker must be idle"
    );
    check_context_fit(
        session.usage.as_ref().and_then(|usage| usage.prompt_tokens),
        target_window,
    )?;
    let pane = tmux::TmuxTarget::window(&tmux::WindowTarget::parse(&meta.window)?);
    let cursor = tmux::cursor_position(&pane)?;
    let screen = tmux::capture_visible_rows(&pane)?;
    ensure!(
        cursor.visible
            && screen.lines().nth(cursor.y).is_some_and(|line| line
                .chars()
                .all(|ch| ch.is_whitespace() || matches!(ch, '❯' | '›' | '>'))),
        "retier requires an empty composer; inspect with niles peek {id}"
    );
    for step in &steps {
        confirm(&pane, step).context("retier unconfirmed; live settings may have partially changed; inspect with niles peek before retrying")?;
    }
    meta.agent = target.canonical();
    meta.agent_family = Some(target.family().to_owned());
    meta.model = Some(model.to_owned());
    meta.effort = Some(effort.to_owned());
    write_meta(&dir, &meta)?;
    println!("retiered: {id} {}", meta.agent);
    Ok(())
}

fn check_context_fit(conversation_tokens: Option<u64>, target_window: u64) -> Result<()> {
    let tokens = conversation_tokens.context("unknown conversation size")?;
    ensure!(
        tokens <= target_window,
        "conversation {tokens} exceeds window {target_window}"
    );
    Ok(())
}

fn confirm(pane: &tmux::TmuxTarget, step: &agents::SwitchStep) -> Result<()> {
    let before = acknowledgements(&tmux::capture_pane(pane, 0)?, &step.acknowledgement);
    tmux::send_input(pane, &step.text, &step.keys)?;
    let deadline = Instant::now() + CONFIRM_TIMEOUT;
    loop {
        if acknowledgements(&tmux::capture_pane(pane, 0)?, &step.acknowledgement) > before {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("CLI did not acknowledge {:?}", step.acknowledgement);
        }
        std::thread::sleep(CONFIRM_INTERVAL);
    }
}

// Pinned CLI handlers above expose no confirmation API. Count value-specific success lines
// in history so a redraw, error, picker or old acknowledgement cannot confirm this command.
fn acknowledgements(screen: &str, reply: &(String, String)) -> usize {
    let prefix = reply.0.to_ascii_lowercase();
    let suffix = reply.1.to_ascii_lowercase();
    screen
        .lines()
        .filter(|line| {
            let line = line
                .trim()
                .trim_start_matches("⎿")
                .trim()
                .to_ascii_lowercase();
            line.strip_prefix(&prefix).is_some_and(|tail| {
                (prefix.is_empty() || tail.is_empty() || tail.starts_with(' '))
                    && line.contains(&suffix)
            })
        })
        .count()
}

#[cfg(test)]
mod tests;
