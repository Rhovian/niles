use super::meta::read_meta;
use super::worker_dir;
use crate::agent_window::shell_quote;
use crate::harness::operations::{InputDisposition, PermissionAnswer};
use crate::harness::{
    Harness, Prompt,
    acp::{self, Headless},
    native::NativeTui,
};
use crate::telemetry::SessionLink;
use crate::tmux::{self, WindowTarget};
use crate::wake::{self, FOLLOWUP_STARTED};
use crate::workspace_manifest::setup::SetupSelection;
use crate::{util, wait, watch, worker};
use anyhow::{Context, Result, ensure};
use camino::Utf8PathBuf;
use chrono::Utc;

/// How far back a bare `niles peek` reads.
///
/// A glance at the pane; `--lines 0` captures the whole history.
pub(crate) const DEFAULT_PEEK_LINES: usize = 200;
struct WorkerPane {
    id: String,
    target: WindowTarget,
    /// The workspace the worker belongs to, where its check-in defaults are read.
    project: Utf8PathBuf,
    headless: bool,
    dir: Utf8PathBuf,
}
impl WorkerPane {
    fn harness(&self) -> Result<Harness<'_>> {
        if self.headless {
            Harness::negotiate(Headless::connect(&self.dir)?)
        } else {
            Harness::negotiate(NativeTui {
                target: tmux::TmuxTarget::window(&self.target),
            })
        }
    }
}
pub fn peek(id: String, lines: usize, quiet: bool) -> Result<()> {
    let target = worker_target(id)?;
    if target.headless {
        acp::peek(&target.dir, lines)?;
    } else {
        print!(
            "{}",
            tmux::capture_pane(&tmux::TmuxTarget::window(&target.target), lines)?
        );
    }
    if quiet {
        if watch::quiet(&target.id)? {
            eprintln!("quiet: {}", target.id);
        } else {
            eprintln!("quiet: {} (no check-in armed)", target.id);
        }
    }
    Ok(())
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
    selection: Option<&SetupSelection>,
) -> Result<()> {
    let target = worker_target(id)?;
    // Resolved before anything is delivered — the wake cursor, the paste — so a manifest typo
    // fails a dispatch that has not happened yet rather than one already typed into the pane.
    let cadence = watch::checkin_cadence(&target.project, checkin.as_deref(), selection)?;
    let dir = &target.dir;
    let mut harness = target.harness()?;
    let id = target.id.as_str();
    for line in wait::advance_cursor(id)? {
        eprintln!("skipped unconsumed wake: {line}");
    }
    // Read before the message is typed: a report that lands while the send is in flight is the
    // worker answering this assignment, and a baseline taken afterwards would swallow it.
    let armed_len = worker::status_log_len(dir)?;
    let dispatched_at = Utc::now();
    // An unreported follow-up stays visible to wait even when check-ins are disabled.
    util::append_line(&wake::status_log_path(dir), FOLLOWUP_STARTED)?;
    if harness.prompt(Prompt(message))? == InputDisposition::Queued {
        eprintln!("queued: {id}");
    }
    // A message to a worker is an assignment, so the lead arms a check-in with it — the same
    // contract `spawn` writes, and the one thing a worker cannot do for itself.
    let armed = watch::arm_checkin(dir, cadence, armed_len, dispatched_at)?;

    println!("sent: {id}");
    match armed {
        Some(delay) => println!("checkin: {}", watch::describe_delay(delay)),
        None => println!("checkin: off"),
    }
    if !wait {
        // wait first, as spawn prints it: the send has armed a wake, and collecting it is the
        // next move. `--wait` is already doing that, so it needs no pointer to itself.
        println!("wait: niles wait");
        println!("peek: niles peek {id}");
    }
    Ok(())
}

fn worker_target(id: String) -> Result<WorkerPane> {
    let dir = worker_dir(&id)?;
    let meta = read_meta(&dir)?;
    let target = WindowTarget::parse(&meta.window)
        .with_context(|| format!("worker {id} metadata has invalid tmux window target"))?;
    Ok(WorkerPane {
        id,
        target,
        project: meta.project,
        dir,
        headless: matches!(meta.session_link, Some(SessionLink::Acp { .. })),
    })
}

pub(crate) fn cancel(id: String) -> Result<()> {
    let target = worker_target(id)?;
    target.harness()?.cancel()?;
    println!("cancel requested: {}", target.id);
    Ok(())
}

pub(crate) fn resume(id: String) -> Result<()> {
    let target = worker_target(id)?;
    let meta = read_meta(&target.dir)?;
    if target.headless
        && matches!(
            super::resolve::window_state(&meta),
            tmux::TargetState::PaneExited
        )
    {
        ensure!(
            acp::capabilities(&target.dir)?.resume,
            "harness does not support resume"
        );
        tmux::resume_window(
            &target.target,
            meta.agent_dir(),
            &format!("sh {}", shell_quote(meta.launch.as_str())),
        )?;
    } else {
        let mut harness = target.harness()?;
        harness.require_resume()?;
        let session = acp::session(&target.dir)?;
        harness.resume(&session)?;
    }
    println!("resumed: {}", target.id);
    Ok(())
}

pub(crate) fn approve(id: String, request: String, option: Option<String>) -> Result<()> {
    let target = worker_target(id)?;
    let answer = option.map_or(PermissionAnswer::Cancelled, PermissionAnswer::Selected);
    target.harness()?.answer_permission(&request, answer)
}
