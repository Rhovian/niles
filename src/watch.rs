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
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, SecondsFormat, Utc};

use crate::{
    tmux::{self, PaneTarget},
    worker::{self, WorkerSnapshot, worker_snapshot},
};

mod checkin;
mod decide;
#[cfg(test)]
mod tests;

use checkin::Checkin;
use decide::{Commit, Plan, WatchMemory};

pub(crate) use checkin::{describe_delay, resolve_delay};

/// How often the workspace is re-read. Report detection is a log-length comparison, so this only
/// has to be prompt enough that a lead who is idle does not stay idle for long.
const TICK_INTERVAL: Duration = Duration::from_secs(1);

/// The tick is slept in slices so stopping the watcher does not wait out a whole one.
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(100);

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
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
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
    if let Err(err) = PaneTarget::pane(pane) {
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
/// Returns the delay armed, or `None` when `--checkin 0`/`off` asked for no check-in at all.
pub(crate) fn arm_checkin(
    worker_dir: &Utf8Path,
    flag: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Option<Duration>> {
    let Some(delay) = resolve_delay(flag)? else {
        return Ok(None);
    };
    Checkin::armed(delay, worker::status_log_len(worker_dir)?, now).write(worker_dir)?;
    Ok(Some(delay))
}

/// `niles quiet <id>`: the lead disarming a check-in by hand, for a worker that is idle on purpose.
/// Returns whether one was armed to begin with.
pub(crate) fn quiet(id: &str) -> Result<bool> {
    Checkin::disarm(&worker::worker_dir(id)?)
}

fn watch(workspace: Utf8PathBuf, log: WatchLog, pane: String, stop: Arc<AtomicBool>) {
    let Ok(target) = PaneTarget::pane(&pane) else {
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
        tick(&workspace, Utc::now(), &mut memory, &mut sink);
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
fn tick(workspace: &Utf8Path, now: DateTime<Utc>, memory: &mut WatchMemory, sink: &mut dyn Sink) {
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
    apply(&plan, memory, sink);
}

fn apply(plan: &Plan, memory: &mut WatchMemory, sink: &mut dyn Sink) {
    for (id, worker_dir) in &plan.disarms {
        match Checkin::disarm(worker_dir) {
            Ok(true) => sink.note(&format!("check-in disarmed: {id} answered it")),
            // Already gone: `niles quiet` got there first, or the worker reported twice.
            Ok(false) => {}
            Err(err) => sink.note(&format!("failed to disarm the check-in for {id}: {err:#}")),
        }
    }

    for nudge in &plan.nudges {
        match sink.nudge(&nudge.text) {
            Ok(()) => {
                // Only a delivered nudge moves the state forward: this is the whole retry
                // mechanism, and it is why a failure here is not an error.
                memory.commit(nudge);
                if let Commit::Checkin(next) = &nudge.commit
                    && let Err(err) = next.write(&nudge.worker_dir)
                {
                    sink.note(&format!(
                        "the check-in for {} is still armed at its old deadline, so it fires \
                         again: {err:#}",
                        nudge.id
                    ));
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
    target: PaneTarget,
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
