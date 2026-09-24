use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::{agent_window, tmux::WindowTarget, wait, watch};

use super::{meta::read_meta, worker_dir};

/// How far back a bare `niles peek` reads.
///
/// A peek lands in the lead's context, so the default is a glance at the pane — a few screens of
/// it — rather than a scrollback dump; `--lines 0` is there when the whole history is wanted. The
/// deep capture belongs to `archive::FINAL_PANE_CAPTURE_LINES`, which writes to a file instead.
pub(crate) const DEFAULT_PEEK_LINES: usize = 200;
enum PaneTarget {
    Worker {
        id: String,
        target: WindowTarget,
        /// The workspace the worker belongs to — where its check-in defaults are read from, which
        /// is the workspace the worker was spawned in and not necessarily the one niles runs in.
        project: Utf8PathBuf,
    },
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
pub fn send(
    wait: bool,
    checkin: Option<String>,
    target_and_message: Vec<String>,
) -> Result<SendOutcome> {
    if target_and_message.is_empty() {
        bail!("send requires a message");
    }

    let (target, message, wait_requested, checkin) =
        resolve_send_target(wait, checkin, target_and_message)?;
    // Resolved before anything is delivered — the wake cursor, the paste — so a manifest typo
    // fails a dispatch that has not happened yet rather than one already typed into the pane.
    let cadence = watch::checkin_cadence(target.project(), checkin.as_deref())?;
    let message = message.join(" ");
    let id = target.label();

    for line in wait::advance_cursor(&id)? {
        eprintln!("skipped unconsumed wake: {line}");
    }

    let dir = worker_dir(&id)?;
    // Read before the message is typed: a report that lands while the send is in flight is the
    // worker answering this assignment, and a baseline taken afterwards would swallow it.
    let armed_len = crate::worker::status_log_len(&dir)?;
    target.send(&message)?;
    // A message to a worker is an assignment, so the lead arms a check-in with it — the same
    // contract `spawn` writes, and the one thing a worker cannot do for itself.
    let armed = watch::arm_checkin(&dir, cadence, armed_len, Utc::now())?;

    println!("sent: {id}");
    match armed {
        Some(delay) => println!("checkin: {}", watch::describe_delay(delay)),
        None => println!("checkin: off"),
    }
    if !wait_requested {
        // wait first, as spawn prints it: the send has armed a wake, and collecting it is the
        // next move. `--wait` is already doing that, so it needs no pointer to itself.
        println!("wait: niles wait {id}");
        println!("peek: niles peek {id}");
    }
    Ok(SendOutcome { id, wait_requested })
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

    fn project(&self) -> &Utf8Path {
        match self {
            PaneTarget::Worker { project, .. } => project,
        }
    }
}

fn resolve_send_target(
    wait: bool,
    checkin: Option<String>,
    target_and_message: Vec<String>,
) -> Result<(PaneTarget, Vec<String>, bool, Option<String>)> {
    let mut parts = target_and_message.into_iter();
    let id = parts.next().context("send requires a worker id")?;
    let mut message = parts.collect::<Vec<_>>();

    // `--wait` or `--checkin <delay>` written after the worker id lands in the trailing message
    // text instead of the flag.
    let (trailing_wait, trailing_checkin) = crate::cli::take_leading_dispatch_flags(&mut message);
    let wait_requested = wait | trailing_wait;
    let checkin = checkin.or(trailing_checkin);

    if message.is_empty() {
        bail!("send requires a message");
    }
    Ok((worker_target(id)?, message, wait_requested, checkin))
}

fn worker_target(id: String) -> Result<PaneTarget> {
    let meta = read_meta(&id)?;
    let target = WindowTarget::parse(&meta.window)
        .with_context(|| format!("worker {id} metadata has invalid tmux window target"))?;
    Ok(PaneTarget::Worker {
        id,
        target,
        project: meta.project,
    })
}
