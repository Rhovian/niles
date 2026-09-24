//! The workspace watcher.
//!
//! Development must not stall waiting for the operator to nudge the lead. The lead idles by
//! design while workers run, and an idle agent TUI only moves when something types into it — so
//! niles is that something. The thread lives in the foreground `niles` process, which runs for
//! exactly as long as the lead, already owns the lead's pane, and is the only writer to it.
//!
//! Two triggers type one line into the lead's pane: a worker's status log grew with an actionable
//! line, or a check-in came due with no report. Both are STATE, not events — the nudge says where
//! things stand, carries no status-line content, and consumes nothing. `niles wait` stays the only
//! consumer of the wake cursor, so a nudge that is lost, doubled or read twice costs a look.
//!
//! The lead's stdout and stderr are its TUI, so nothing here writes to them: diagnostics go to
//! `watch.log` in the session directory, and the only thing that leaves the process is the nudge
//! itself, through the same `send_line` that `niles send` uses.

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, SecondsFormat, Utc};

use crate::{
    tmux::{self, TmuxTarget},
    worker::{self, WorkerSnapshot, worker_snapshot},
};

mod checkin;
mod decide;
#[cfg(test)]
mod tests;

use checkin::Checkin;
use decide::{Commit, Nudge, Plan, WatchMemory};

pub(crate) use checkin::{describe_delay, resolve_delay};

/// How often the workspace is re-read. Report detection is a log-length comparison, so this only
/// has to be prompt enough that a lead who is idle does not stay idle for long.
const TICK_INTERVAL: Duration = Duration::from_secs(1);

/// The tick is slept in slices so stopping the watcher does not wait out a whole one.
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// How long dropping the watcher waits for the thread.
///
/// A tick can be inside a `send_line` that is waiting on the lead's pane to redraw — up to its
/// settle plus submit timeouts — and the lead's exit must not queue behind tmux. One send in flight
/// is given its full time to land, so a nudge is not abandoned half-typed; past that, an exit that
/// waits is worse than a nudge that is collected from the state on disk next session.
const EXIT_JOIN_TIMEOUT: Duration = Duration::from_secs(10);

const WATCH_LOG: &str = "watch.log";

/// A running watcher. Dropping it stops and joins the thread, which is what ties the thread's
/// lifetime to the lead's: `launch_foreground_agent` holds one for as long as the foreground agent
/// runs, on the normal path and on the failing one.
pub(crate) struct Watcher {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Watcher {
    /// No pane to type into, so no thread: niles outside tmux stays fully usable.
    fn idle() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            handle: None,
        }
    }

    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let Some(handle) = self.handle.take() else {
            return;
        };
        // The wait is bounded by handing the join to a thread nobody keeps: `join` itself has no
        // timeout, and a watcher wedged in tmux would otherwise hold the lead's exit forever.
        let (joined, waiter) = mpsc::channel();
        thread::spawn(move || {
            let _ = handle.join();
            let _ = joined.send(());
        });
        let _ = waiter.recv_timeout(EXIT_JOIN_TIMEOUT);
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

/// Starts the watcher for the lead's lifetime.
///
/// The pane is the one the session recorded at startup — a `%N` pane id, which tmux accepts as a
/// complete target. A session that recorded none (niles was run outside tmux) has no pane to type
/// into, and says so in one line rather than starting a thread that cannot nudge anybody.
pub(crate) fn start(
    session_dir: &Utf8Path,
    workspace: &Utf8Path,
    lead_pane: Option<&str>,
) -> Watcher {
    let log = WatchLog::new(session_dir.join(WATCH_LOG));
    let Some(pane) = lead_pane.map(str::trim).filter(|pane| !pane.is_empty()) else {
        log.note(
            "watcher not started: this session recorded no lead pane, so there is nothing to type \
             into; niles works without it",
        );
        return Watcher::idle();
    };
    if let Err(err) = TmuxTarget::pane(pane) {
        log.note(&format!("watcher not started: {err:#}"));
        return Watcher::idle();
    }

    let stop = Arc::new(AtomicBool::new(false));
    let workspace = workspace.to_path_buf();
    let spawned = thread::Builder::new()
        .name("niles-watch".to_owned())
        .spawn({
            let stop = Arc::clone(&stop);
            let pane = pane.to_owned();
            let log = log.clone();
            move || watch(workspace, log, pane, stop)
        });

    match spawned {
        Ok(handle) => Watcher {
            stop,
            handle: Some(handle),
        },
        Err(err) => {
            log.note(&format!(
                "watcher not started: failed to spawn the thread: {err}"
            ));
            Watcher::idle()
        }
    }
}

/// Arms a worker's check-in, as `spawn` and `send` do: the lead is the only one who arms one.
///
/// `armed_len` is the status log's length *before* the work was dispatched — the caller reads it
/// before the message is typed or the window launched, because a line written while that happens is
/// the worker answering this assignment, and a length read afterwards would fold it into the
/// baseline and leave it unable to answer anything.
///
/// Returns the delay armed, or `None` when `--checkin 0`/`off` asked for no check-in at all.
pub(crate) fn arm_checkin(
    worker_dir: &Utf8Path,
    flag: Option<&str>,
    armed_len: u64,
    now: DateTime<Utc>,
) -> Result<Option<Duration>> {
    let Some(delay) = resolve_delay(flag)? else {
        // `off`/`0` asks for no check-in and has to mean it: leaving the previous one armed would
        // make the `checkin: off` the lead was just printed a lie.
        Checkin::disarm(worker_dir)?;
        return Ok(None);
    };
    Checkin::armed(delay, armed_len, now).write(worker_dir)?;
    Ok(Some(delay))
}

/// `niles quiet <id>`: the lead disarming a check-in by hand, for a worker that is idle on purpose.
/// Returns whether one was armed to begin with.
pub(crate) fn quiet(id: &str) -> Result<bool> {
    Checkin::disarm(&worker::worker_dir(id)?)
}

fn watch(workspace: Utf8PathBuf, log: WatchLog, pane: String, stop: Arc<AtomicBool>) {
    let Ok(target) = TmuxTarget::pane(&pane) else {
        log.note(&format!("watcher not started: invalid pane id `{pane}`"));
        return;
    };
    let mut sink = WatchSink { target, log };
    let mut memory = match worker_snapshot(&workspace) {
        Ok(snapshot) => WatchMemory::at_start(&snapshot),
        Err(err) => {
            sink.note(&format!(
                "the first worker snapshot failed ({err:#}); workers appearing from now on are \
                 read from the start of their log"
            ));
            WatchMemory::default()
        }
    };
    sink.note(&format!(
        "watcher started: pane={pane} workspace={workspace}"
    ));

    while !stop.load(Ordering::Relaxed) {
        tick(&workspace, Utc::now(), &mut memory, &mut sink, &stop);
        sleep_until_next_tick(&stop);
    }
    sink.note("watcher stopped: the foreground agent exited");
}

fn sleep_until_next_tick(stop: &AtomicBool) {
    let deadline = Instant::now() + TICK_INTERVAL;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        thread::sleep(STOP_POLL_INTERVAL);
    }
}

/// One pass: read the workspace, decide, deliver what the decision asked for.
fn tick(
    workspace: &Utf8Path,
    now: DateTime<Utc>,
    memory: &mut WatchMemory,
    sink: &mut dyn Sink,
    stop: &AtomicBool,
) {
    let snapshot = match worker_snapshot(workspace) {
        Ok(snapshot) => snapshot,
        Err(err) => {
            sink.note(&format!(
                "failed to read the workspace ({err:#}); retrying next tick"
            ));
            return;
        }
    };
    let checkins = read_checkins(&snapshot, sink);
    let plan = memory.plan(&snapshot, &checkins, now);
    apply(&plan, memory, sink, stop);
}

/// Delivers a plan, one nudge at a time.
///
/// Every file this touches is re-read first: a tick spends seconds inside `send_line`, and anything
/// armed or answered in that window is newer than the plan and is left for the next tick to judge.
/// Nothing here is worth losing — a lost nudge costs a look.
fn apply(plan: &Plan, memory: &mut WatchMemory, sink: &mut dyn Sink, stop: &AtomicBool) {
    for disarm in &plan.disarms {
        with_planned_checkin(
            &disarm.id,
            &disarm.worker_dir,
            &disarm.planned,
            sink,
            |worker_dir, sink| match Checkin::disarm(worker_dir) {
                Ok(true) => sink.note(&format!("check-in disarmed: {} answered it", disarm.id)),
                // Gone between the re-read and the remove.
                Ok(false) => {}
                Err(err) => sink.note(&format!(
                    "failed to disarm the check-in for {}: {err:#}",
                    disarm.id
                )),
            },
        );
    }

    for nudge in &plan.nudges {
        // The lead is exiting. What is left of the plan stays undelivered and uncommitted, and the
        // state it described is still on disk for the next session.
        if stop.load(Ordering::Relaxed) {
            sink.note("the foreground agent is exiting: the rest of this plan waits for next time");
            return;
        }
        match sink.nudge(&nudge.text) {
            Ok(()) => {
                // Only a delivered nudge moves the state forward: this is the whole retry
                // mechanism, and it is why a failure here is not an error.
                memory.commit(nudge);
                if let Commit::Checkin { planned, next } = &nudge.commit {
                    rearm(nudge, planned, next, sink);
                }
                sink.note(&format!("nudged {}: {}", nudge.id, nudge.text));
            }
            Err(err) => sink.note(&format!(
                "the nudge for {} did not land, retrying next tick: {err:#}",
                nudge.id
            )),
        }
    }
}

/// Writes the next check-in, but only over the state this tick planned on.
fn rearm(nudge: &Nudge, planned: &Checkin, next: &Checkin, sink: &mut dyn Sink) {
    with_planned_checkin(
        &nudge.id,
        &nudge.worker_dir,
        planned,
        sink,
        |worker_dir, sink| {
            if let Err(err) = next.write(worker_dir) {
                sink.note(&format!(
                    "the check-in for {} is still armed at its old deadline, so it fires again: \
                     {err:#}",
                    nudge.id
                ));
            }
        },
    );
}

/// Runs `action` on a check-in only when the file is still the state the plan was built from.
///
/// A tick spends seconds inside `send_line`, so an arm state written in that window is newer than
/// the plan and belongs to work the tick has not seen: the action is skipped, and the reason is
/// logged here once rather than in each caller.
fn with_planned_checkin<S, F>(
    id: &str,
    worker_dir: &Utf8Path,
    planned: &Checkin,
    sink: &mut S,
    action: F,
) where
    S: Sink + ?Sized,
    F: FnOnce(&Utf8Path, &mut S),
{
    match Checkin::read(worker_dir) {
        Ok(Some(current)) if current == *planned => action(worker_dir, sink),
        // Nothing armed: `niles quiet` got there first, or the worker reported twice.
        Ok(None) => {}
        Ok(Some(current)) => sink.note(&format!(
            "left the check-in for {id} alone: it was re-armed while this tick was delivering \
             (armed_len {} where this tick planned on {})",
            current.armed_len, planned.armed_len
        )),
        Err(err) => sink.note(&format!("failed to re-read the check-in for {id}: {err:#}")),
    }
}

fn read_checkins(snapshot: &[WorkerSnapshot], sink: &mut dyn Sink) -> BTreeMap<String, Checkin> {
    let mut checkins = BTreeMap::new();
    for worker in snapshot {
        match Checkin::read(&worker.worker_dir) {
            Ok(Some(checkin)) => {
                checkins.insert(worker.id.clone(), checkin);
            }
            Ok(None) => {}
            // Unreadable arm state disarms nothing and nudges nothing: the next tick reads it
            // again, and a checker that guessed at half a file would be worse than one that waits.
            Err(err) => sink.note(&format!(
                "ignoring the unreadable check-in for {}: {err:#}",
                worker.id
            )),
        }
    }
    checkins
}

/// The edge: how a nudge leaves this process, and where a diagnostic goes.
trait Sink {
    fn nudge(&mut self, text: &str) -> Result<()>;
    fn note(&mut self, line: &str);
}

struct WatchSink {
    target: TmuxTarget,
    log: WatchLog,
}

impl Sink for WatchSink {
    fn nudge(&mut self, text: &str) -> Result<()> {
        // Deliberately not `nudge_line` or a hand-rolled send-keys: `send_line` is the one place
        // that knows how to confirm a submit took, and a nudge that silently sat in the lead's
        // composer is the failure this whole feature exists to avoid.
        tmux::send_line(&self.target, text)
    }

    fn note(&mut self, line: &str) {
        self.log.note(line);
    }
}

/// The watcher's own record of what it did, appended one line at a time.
#[derive(Clone)]
struct WatchLog {
    path: Utf8PathBuf,
}

impl WatchLog {
    fn new(path: Utf8PathBuf) -> Self {
        Self { path }
    }

    fn note(&self, line: &str) {
        let stamp = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        // A log that cannot be appended is not worth stopping the watcher over: the nudge is the
        // feature, and there is nowhere else this failure could be reported to.
        let _ = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut file| writeln!(file, "{stamp} {line}"));
    }
}
