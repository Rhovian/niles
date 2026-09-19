use anyhow::{Context, Result, bail};

use crate::{agent_window, tmux::WindowTarget, wait};

use super::meta::read_meta;

pub(crate) const DEFAULT_PEEK_LINES: usize = 2000;
const WAIT_FLAG: &str = "--wait";
enum PaneTarget {
    Worker { id: String, target: WindowTarget },
}
pub fn peek(id: String, lines: usize) -> Result<()> {
    let target = worker_target(id)?;
    print!("{}", target.capture(lines)?);
    Ok(())
}

/// What a send resolved to: the worker it reached, and whether the caller asked to block.
pub struct SendOutcome {
    pub id: String,
    pub wait_requested: bool,
}

/// Sends a message to a worker pane.
///
/// The wake cursor is advanced first, so the wait that follows this send cannot be satisfied by a
/// status line the worker wrote before the message arrived.
pub fn send(target_and_message: Vec<String>) -> Result<SendOutcome> {
    if target_and_message.is_empty() {
        bail!("send requires a message");
    }

    let (target, message, wait_requested) = resolve_send_target(target_and_message)?;
    let message = message.join(" ");
    let id = target.label();

    for line in wait::advance_cursor(&id)? {
        eprintln!("skipped unconsumed wake: {line}");
    }

    target.send(&message)?;
    println!("sent: {id}");
    Ok(SendOutcome {
        id,
        wait_requested,
    })
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

fn resolve_send_target(
    target_and_message: Vec<String>,
) -> Result<(PaneTarget, Vec<String>, bool)> {
    let mut parts = target_and_message.into_iter();
    let id = parts.next().context("send requires a worker id")?;
    let mut message = parts.collect::<Vec<_>>();

    // The message is a trailing var-arg, so clap hands `--wait` to us as message text when it is
    // written after the worker id — the order most people reach for. Honor it there rather than
    // typing `--wait` into the agent's pane.
    let wait_requested = message.first().is_some_and(|first| first == WAIT_FLAG);
    if wait_requested {
        message.remove(0);
    }

    if message.is_empty() {
        bail!("send requires a message");
    }
    Ok((worker_target(id)?, message, wait_requested))
}

fn worker_target(id: String) -> Result<PaneTarget> {
    let meta = read_meta(&id)?;
    let target = WindowTarget::parse(&meta.window)
        .with_context(|| format!("worker {id} metadata has invalid tmux window target"))?;
    Ok(PaneTarget::Worker { id, target })
}
