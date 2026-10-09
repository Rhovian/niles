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
    io::{ErrorKind, Write},
    panic,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, SecondsFormat, Utc};

use crate::{
    tmux::{self, TmuxTarget},
    wake::WakeKind,
    worker::{self, WorkerSnapshot, worker_snapshot},
    workspace_manifest,
};

mod cadence;
mod checkin;
pub(crate) mod composer;
mod decide;
mod due;
#[cfg(test)]
mod tests;
mod trust;

use cadence::Cadence;
pub(crate) use checkin::Checkin;
use decide::{Commit, Nudge, Plan, WatchMemory, parse_report_text};

pub(crate) use cadence::{DEFAULT_DELAY, Recheck, describe_delay, parse_delay, parse_recheck};

/// How often the workspace is re-read. Report detection is a log-length comparison, so this only
/// has to be prompt enough that a lead who is idle does not stay idle for long.
const TICK_INTERVAL: Duration = Duration::from_secs(1);

/// The tick is slept in slices so stopping the watcher does not wait out a whole one.
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// One in-flight `send_line` gets its settle+submit time to land; past that, exiting beats finishing
/// the nudge.
const EXIT_JOIN_TIMEOUT: Duration = Duration::from_secs(10);

const WATCH_LOG: &str = "watch.log";
const WATCH_THREAD_NAME: &str = "niles-watch";

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
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            join_watcher(handle);
        }
    }
}

fn join_watcher(handle: JoinHandle<()>) {
    let (joined, waiter) = mpsc::channel();
    thread::spawn(move || {
        let _ = handle.join();
        let _ = joined.send(());
    });
    let _ = waiter.recv_timeout(EXIT_JOIN_TIMEOUT);
}

fn install_panic_hook(log: WatchLog) {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if thread::current().name() == Some(WATCH_THREAD_NAME) {
            log.note(&format!("watcher panic: {info}"));
        } else {
            previous(info);
        }
    }));
}

#[cfg(test)]
mod panic_tests {
    use super::*;
    use crate::test_support::temp_test_path;

    #[test]
    fn a_panicking_watcher_is_recorded_by_its_hook() {
        let session = temp_test_path("panic-session");
        fs::create_dir_all(&session).unwrap();
        let log = WatchLog {
            path: session.join(WATCH_LOG),
        };
        let handle = thread::Builder::new()
            .name(WATCH_THREAD_NAME.to_owned())
            .spawn(move || {
                install_panic_hook(log);
                panic!("broken watcher");
            })
            .unwrap();

        join_watcher(handle);

        let body = fs::read_to_string(session.join(WATCH_LOG)).unwrap();
        assert!(body.contains("watcher panic:"), "{body}");
        assert!(body.contains("broken watcher"), "{body}");
        fs::remove_dir_all(&session).unwrap();
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
    composer: Option<&'static str>,
) -> Watcher {
    let log = WatchLog {
        path: session_dir.join(WATCH_LOG),
    };
    let Some(pane) = lead_pane else {
        log.note(
            "watcher not started: this session recorded no lead pane, so there is nothing to type \
             into; niles works without it",
        );
        return Watcher::idle();
    };
    let target = match TmuxTarget::pane(pane) {
        Ok(target) => target,
        Err(err) => {
            log.note(&format!("watcher not started: {err:#}"));
            return Watcher::idle();
        }
    };

    let sink = WatchSink {
        target,
        log: log.clone(),
        composer,
    };
    let stop = Arc::new(AtomicBool::new(false));
    let workspace = workspace.to_path_buf();
    let spawned = thread::Builder::new()
        .name(WATCH_THREAD_NAME.to_owned())
        .spawn({
            let stop = Arc::clone(&stop);
            let log = log.clone();
            move || {
                install_panic_hook(log.clone());
                watch(workspace, sink, stop);
            }
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

/// The check-in a dispatch arms, resolved from the `--checkin` flag and the workspace manifest.
///
/// Read here rather than at the arming edge because the two moments differ: this is resolved
/// before anything is dispatched, and armed after. A workspace without a manifest has no
/// defaults to apply, which is not an error — `niles spawn --agent` works without one.
pub(crate) fn checkin_cadence(project: &Utf8Path, flag: Option<&str>) -> Result<Cadence> {
    let path = workspace_manifest::manifest_path(project);
    let manifest = workspace_manifest::load(project)
        .with_context(|| format!("cannot resolve the check-in from {path}"))?;
    cadence::resolve_cadence(flag, manifest.as_ref(), &path)
}

/// Arms a worker's check-in, as `spawn` and `send` do: the lead is the only one who arms one.
///
/// `armed_len` is the status log's length *before* the work was dispatched — the caller reads it
/// before the message is typed or the window launched, because a line written while that happens is
/// the worker answering this assignment, and a length read afterwards would fold it into the
/// baseline and leave it unable to answer anything.
///
/// Returns the delay armed, or `None` when the cadence asked for no check-in at all.
pub(crate) fn arm_checkin(
    worker_dir: &Utf8Path,
    cadence: Cadence,
    armed_len: u64,
    now: DateTime<Utc>,
) -> Result<Option<Duration>> {
    let Some(delay) = cadence.delay else {
        // `off`/`0` asks for no check-in and has to mean it: leaving the previous one armed would
        // make the `checkin: off` the lead was just printed a lie.
        Checkin::disarm(worker_dir)?;
        return Ok(None);
    };
    Checkin::armed(delay, cadence.recheck, armed_len, now).write(worker_dir)?;
    Ok(Some(delay))
}

/// `niles quiet <id>`: the lead disarming a check-in by hand, for a worker that is idle on purpose.
/// Returns whether one was armed to begin with.
pub(crate) fn quiet(id: &str) -> Result<bool> {
    Checkin::disarm(&worker::worker_dir(id)?)
}

fn watch(workspace: Utf8PathBuf, mut sink: WatchSink, stop: Arc<AtomicBool>) {
    let pane = sink.target.as_str().to_owned();
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
    trust::inspect_starting_workers(&snapshot, now, sink);
    let mut checkins = read_checkins(&snapshot, sink);
    due::check_due(&snapshot, &mut checkins, now, memory, sink);
    let mut plan = memory.plan(&snapshot, &checkins, now);
    let has_draft = !plan.nudges.is_empty() && sink.has_draft();
    if memory.hold_nudge(has_draft, now) {
        plan.nudges.clear();
    }
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
fn with_planned_checkin(
    id: &str,
    worker_dir: &Utf8Path,
    planned: &Checkin,
    sink: &mut dyn Sink,
    action: impl FnOnce(&Utf8Path, &mut dyn Sink),
) {
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

/// The edge: read worker panes, send lead nudges, and record diagnostics without using stdout.
trait Sink {
    fn nudge(&mut self, text: &str) -> Result<()>;
    fn has_draft(&mut self) -> bool;
    fn capture_visible(&mut self, target: &TmuxTarget) -> Result<String>;
    fn note(&mut self, line: &str);
}

struct WatchSink {
    target: TmuxTarget,
    log: WatchLog,
    composer: Option<&'static str>,
}

impl Sink for WatchSink {
    fn has_draft(&mut self) -> bool {
        let Some(marker) = self.composer else {
            return false;
        };
        let screen = tmux::capture_visible_rows(&self.target);
        let cursor = tmux::cursor_position(&self.target);
        match (screen, cursor) {
            (Ok(screen), Ok(cursor)) => composer::recognize(&screen, cursor, marker),
            (Err(err), _) | (_, Err(err)) => {
                self.note(&format!("could not read lead composer: {err:#}"));
                false
            }
        }
    }
    fn nudge(&mut self, text: &str) -> Result<()> {
        // Deliberately not `nudge_line` or a hand-rolled send-keys: `send_line` is the one place
        // that knows how to confirm a submit took, and a nudge that silently sat in the lead's
        // composer is the failure this whole feature exists to avoid.
        tmux::send_line(&self.target, text)
    }

    fn capture_visible(&mut self, target: &TmuxTarget) -> Result<String> {
        tmux::capture_visible_pane(target)
    }

    fn note(&mut self, line: &str) {
        self.log.note(line);
    }
}

/// The report nudges a session's `watch.log` records as delivered, with when each was.
pub(crate) fn delivered_reports(
    session_dir: &Utf8Path,
) -> Result<Vec<(DateTime<Utc>, String, WakeKind)>> {
    let path = session_dir.join(WATCH_LOG);
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("failed to read {path}")),
    };
    #[expect(clippy::disallowed_methods, reason = "other notes are not reports")]
    let reports = body.lines().filter_map(|line| {
        let (stamp, note) = line.split_once(' ')?;
        let (_, text) = note.strip_prefix("nudged ")?.split_once(": ")?;
        let (id, kind) = parse_report_text(text)?;
        Some((
            DateTime::parse_from_rfc3339(stamp).ok()?.to_utc(),
            id.to_owned(),
            kind,
        ))
    });
    Ok(reports.collect())
}

/// The watcher's own record of what it did, appended one line at a time.
#[derive(Clone)]
struct WatchLog {
    path: Utf8PathBuf,
}

impl WatchLog {
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
