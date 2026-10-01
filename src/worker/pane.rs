use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use chrono::Utc;

use crate::{
    tmux::{self, WindowTarget},
    wait, watch,
};

use super::{meta::read_meta, worker_dir};

/// How far back a bare `niles peek` reads.
///
/// A glance at the pane; `--lines 0` captures the whole history.
pub(crate) const DEFAULT_PEEK_LINES: usize = 200;
struct WorkerPane {
    id: String,
    target: WindowTarget,
    /// The workspace the worker belongs to, where its check-in defaults are read.
    project: Utf8PathBuf,
}
pub fn peek(id: String, lines: usize) -> Result<()> {
    let target = worker_target(id)?;
    print!(
        "{}",
        tmux::capture_pane(&tmux::TmuxTarget::window(&target.target), lines)?
    );
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
    id: String,
    message: String,
) -> Result<SendOutcome> {
    let target = worker_target(id)?;
    // Resolved before anything is delivered — the wake cursor, the paste — so a manifest typo
    // fails a dispatch that has not happened yet rather than one already typed into the pane.
    let cadence = watch::checkin_cadence(&target.project, checkin.as_deref())?;
    let id = target.id;

    for line in wait::advance_cursor(&id)? {
        eprintln!("skipped unconsumed wake: {line}");
    }

    let dir = worker_dir(&id)?;
    // Read before the message is typed: a report that lands while the send is in flight is the
    // worker answering this assignment, and a baseline taken afterwards would swallow it.
    let armed_len = crate::worker::status_log_len(&dir)?;
    tmux::send_line(&tmux::TmuxTarget::window(&target.target), &message)?;
    // A message to a worker is an assignment, so the lead arms a check-in with it — the same
    // contract `spawn` writes, and the one thing a worker cannot do for itself.
    let armed = watch::arm_checkin(&dir, cadence, armed_len, Utc::now())?;

    println!("sent: {id}");
    match armed {
        Some(delay) => println!("checkin: {}", watch::describe_delay(delay)),
        None => println!("checkin: off"),
    }
    if !wait {
        // wait first, as spawn prints it: the send has armed a wake, and collecting it is the
        // next move. `--wait` is already doing that, so it needs no pointer to itself.
        println!("wait: niles wait {id}");
        println!("peek: niles peek {id}");
    }
    Ok(SendOutcome {
        id,
        wait_requested: wait,
    })
}

fn worker_target(id: String) -> Result<WorkerPane> {
    let meta = read_meta(&id)?;
    let target = WindowTarget::parse(&meta.window)
        .with_context(|| format!("worker {id} metadata has invalid tmux window target"))?;
    Ok(WorkerPane {
        id,
        target,
        project: meta.project,
    })
}
