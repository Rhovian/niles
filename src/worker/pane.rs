use anyhow::{Context, Result, bail};

use crate::{agent_window, tmux::WindowTarget};

use super::meta::read_meta;

pub(crate) const DEFAULT_PEEK_LINES: usize = 2000;
enum PaneTarget {
    Worker { id: String, target: WindowTarget },
}
pub fn peek(id: String, lines: usize) -> Result<()> {
    let target = worker_target(id)?;
    print!("{}", target.capture(lines)?);
    Ok(())
}

pub fn send(target_and_message: Vec<String>) -> Result<()> {
    if target_and_message.is_empty() {
        bail!("send requires a message");
    }

    let (target, message) = resolve_send_target(target_and_message)?;
    let message = message.join(" ");
    target.send(&message)?;
    println!("sent: {}", target.label());
    Ok(())
}

impl PaneTarget {
    fn capture(&self, lines: usize) -> Result<String> {
        match self {
            PaneTarget::Worker { target, .. } => agent_window::capture_target(target, lines),
        }
    }

    fn send(&self, message: &str) -> Result<()> {
        match self {
            PaneTarget::Worker { target, .. } => agent_window::send_target(target, message),
        }
    }

    fn label(&self) -> String {
        match self {
            PaneTarget::Worker { id, .. } => id.clone(),
        }
    }
}

fn resolve_send_target(target_and_message: Vec<String>) -> Result<(PaneTarget, Vec<String>)> {
    let mut parts = target_and_message.into_iter();
    let id = parts.next().context("send requires a worker id")?;
    let message = parts.collect::<Vec<_>>();
    if message.is_empty() {
        bail!("send requires a message");
    }
    Ok((worker_target(id)?, message))
}

fn worker_target(id: String) -> Result<PaneTarget> {
    let meta = read_meta(&id)?;
    let target = WindowTarget::parse(&meta.window)
        .with_context(|| format!("worker {id} metadata has invalid tmux window target"))?;
    Ok(PaneTarget::Worker { id, target })
}
