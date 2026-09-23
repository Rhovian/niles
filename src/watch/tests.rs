use std::{fs, sync::atomic::AtomicBool, time::Duration};

use anyhow::{Result, bail};
use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};

use crate::worker::worker_snapshot;

use super::{Sink, WatchMemory, apply, arm_checkin, checkin::Checkin, quiet, read_checkins, tick};

/// A watcher that is still running: what every tick test but the stopping one wants.
fn running() -> AtomicBool {
    AtomicBool::new(false)
}

/// A sink that records what the watcher asked of it, so a tick can be driven without tmux.
#[derive(Debug, Default)]
struct RecordingSink {
    attempts: Vec<String>,
    sent: Vec<String>,
    notes: Vec<String>,
    fail: bool,
}

impl Sink for RecordingSink {
    fn nudge(&mut self, text: &str) -> Result<()> {
        self.attempts.push(text.to_owned());
        if self.fail {
            bail!("stub tmux is not answering");
        }
        self.sent.push(text.to_owned());
        Ok(())
    }

    fn note(&mut self, line: &str) {
        self.notes.push(line.to_owned());
    }
}

fn workspace(label: &str) -> Utf8PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
        "niles-watch-{label}-{}-{nanos}",
        std::process::id()
    )))
    .unwrap();
    fs::create_dir_all(&path).unwrap();
    path
}

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(seconds, 0).unwrap()
}

fn worker_dir(workspace: &Utf8PathBuf, id: &str) -> Utf8PathBuf {
    workspace.join(".niles/worker").join(id)
}

/// A live worker as `worker_snapshot` sees one: a directory with metadata and a status log.
fn write_worker(workspace: &Utf8PathBuf, id: &str, log: &str) {
    let dir = worker_dir(workspace, id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("status.log"), log).unwrap();
    fs::write(
        dir.join("meta.json"),
        format!(
            "{{\"niles_schema\":2,\"id\":\"{id}\",\"agent\":\"claude\",\"project\":\"{workspace}\",\
             \"window\":\"niles-test:niles-{id}\",\"brief\":\"{workspace}/brief.md\",\
             \"launch\":\"{workspace}/launch.sh\"}}"
        ),
    )
    .unwrap();
}

fn append_log(workspace: &Utf8PathBuf, id: &str, line: &str) {
    let path = worker_dir(workspace, id).join("status.log");
    let body = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{body}{line}")).unwrap();
}

const WINDOW: &str = "working: launch\n";
const DONE: &str = "done: shipped\n";

#[test]
fn a_failed_nudge_is_retried_on_the_next_tick() {
    let root = workspace("retry");
    write_worker(&root, "impl", WINDOW);
    let now = at(1_000);
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    append_log(&root, "impl", DONE);

    let mut failing = RecordingSink {
        fail: true,
        ..RecordingSink::default()
    };
    tick(&root, now, &mut memory, &mut failing, &running());
    assert_eq!(failing.attempts.len(), 1);
    assert!(failing.sent.is_empty(), "a failed nudge is not a nudge");

    let mut working = RecordingSink::default();
    tick(&root, at(1001), &mut memory, &mut working, &running());
    assert_eq!(
        working.sent,
        vec!["niles: impl reported (done) — check workers"]
    );

    // The report has been handed over, so the third tick has nothing to say about it.
    let mut quiet_tick = RecordingSink::default();
    tick(&root, at(1002), &mut memory, &mut quiet_tick, &running());
    assert!(quiet_tick.attempts.is_empty(), "{quiet_tick:?}");

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_tick_deletes_the_arm_state_when_the_worker_answers() {
    let root = workspace("disarm");
    write_worker(&root, "impl", WINDOW);
    let now = at(1_000);
    let dir = worker_dir(&root, "impl");
    Checkin::armed(Duration::from_secs(300), WINDOW.len() as u64, now)
        .write(&dir)
        .unwrap();
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    append_log(&root, "impl", DONE);

    let mut sink = RecordingSink::default();
    tick(&root, at(1060), &mut memory, &mut sink, &running());

    assert!(!dir.join("checkin").exists(), "the arm state must go");
    assert_eq!(
        sink.sent,
        vec!["niles: impl reported (done) — check workers"]
    );

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_tick_rearms_a_fired_check_in_on_disk() {
    let root = workspace("rearm");
    write_worker(&root, "impl", WINDOW);
    let now = at(1_000);
    let dir = worker_dir(&root, "impl");
    Checkin::armed(Duration::from_secs(300), WINDOW.len() as u64, now)
        .write(&dir)
        .unwrap();
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());

    let mut sink = RecordingSink::default();
    tick(&root, at(1300), &mut memory, &mut sink, &running());

    assert_eq!(
        sink.sent,
        vec!["niles: no report from impl in 5m — check it"]
    );
    // Written back at +3 minutes with the step advanced, so the same check-in does not fire again
    // on the next tick.
    let rearmed = Checkin::read(&dir).unwrap().unwrap();
    assert_eq!(rearmed.deadline, at(1_480));
    assert_eq!(rearmed.minutes(), 8);

    let mut next = RecordingSink::default();
    tick(&root, at(1301), &mut memory, &mut next, &running());
    assert!(next.attempts.is_empty(), "{next:?}");

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_failed_check_in_nudge_leaves_the_check_in_armed_and_overdue() {
    let root = workspace("rearm-failed");
    write_worker(&root, "impl", WINDOW);
    let now = at(1_000);
    let dir = worker_dir(&root, "impl");
    Checkin::armed(Duration::from_secs(300), WINDOW.len() as u64, now)
        .write(&dir)
        .unwrap();
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());

    let mut failing = RecordingSink {
        fail: true,
        ..RecordingSink::default()
    };
    tick(&root, at(1300), &mut memory, &mut failing, &running());

    assert_eq!(Checkin::read(&dir).unwrap().unwrap().deadline, at(1_300));

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn arming_a_check_in_records_the_log_length_and_quiet_disarms_it() {
    let root = workspace("arm-quiet");
    write_worker(&root, "impl", &format!("{WINDOW}{DONE}"));
    let dir = worker_dir(&root, "impl");
    let baseline = (WINDOW.len() + DONE.len()) as u64;

    let armed = arm_checkin(&dir, Some("90s"), baseline, at(1_000)).unwrap();

    assert_eq!(armed, Some(Duration::from_secs(90)));
    let checkin = Checkin::read(&dir).unwrap().unwrap();
    // The length the caller read before dispatching, not one read here: that is what lets a report
    // landing in the meantime answer the assignment it belongs to.
    assert_eq!(checkin.armed_len, baseline);
    assert_eq!(checkin.deadline, at(1_090));

    // `--checkin off` is no check-in, and it takes an armed one with it rather than leaving the
    // lead with a printed `checkin: off` over a live deadline.
    assert_eq!(
        arm_checkin(&dir, Some("off"), baseline, at(1_000)).unwrap(),
        None
    );
    assert_eq!(Checkin::read(&dir).unwrap(), None);

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn quiet_is_a_lead_command_and_needs_a_worker_that_exists() {
    let err = quiet("no-such-worker").unwrap_err();

    assert!(err.to_string().contains("unknown worker id"), "{err}");
}

/// Outside tmux there is no pane to type into: no thread at all, one line saying so, and every
/// other thing niles does is untouched.
#[test]
fn a_session_with_no_lead_pane_starts_no_watcher() {
    let session = workspace("no-pane-session");
    let root = workspace("no-pane-workspace");
    write_worker(&root, "impl", &format!("{WINDOW}{DONE}"));

    drop(super::start(&session, &root, None));

    let log = fs::read_to_string(session.join("watch.log")).unwrap();
    assert!(log.contains("recorded no lead pane"), "{log}");
    assert!(!log.contains("watcher started"), "{log}");

    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&session).unwrap();
}

/// The thread's lifetime is the foreground process's lifetime: dropping the watcher stops and
/// joins it, and the workspace here has no workers, so the tick that may run touches no tmux pane.
#[test]
fn the_watcher_starts_on_the_recorded_pane_and_stops_with_the_process() {
    let session = workspace("pane-session");
    let root = workspace("pane-workspace");

    let watcher = super::start(&session, &root, Some("%7"));
    drop(watcher);

    let log = fs::read_to_string(session.join("watch.log")).unwrap();
    assert!(log.contains("watcher started: pane=%7"), "{log}");
    assert!(
        log.contains("watcher stopped: the foreground agent exited"),
        "{log}"
    );

    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&session).unwrap();
}

/// A lead that is exiting does not wait for the plan: anything not yet typed stays untyped and
/// uncommitted, and the state it described is still on disk for next time.
#[test]
fn a_stopping_tick_delivers_nothing_and_leaves_the_state_for_next_time() {
    let root = workspace("stopping");
    write_worker(&root, "impl", WINDOW);
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    append_log(&root, "impl", DONE);

    let stop = AtomicBool::new(true);
    let mut stopping = RecordingSink::default();
    tick(&root, at(1_000), &mut memory, &mut stopping, &stop);
    assert!(stopping.attempts.is_empty(), "{stopping:?}");

    // Nothing was recorded as delivered, so the next watcher picks the report up.
    let mut later = RecordingSink::default();
    tick(&root, at(1_001), &mut memory, &mut later, &running());
    assert_eq!(
        later.sent,
        vec!["niles: impl reported (done) — check workers"]
    );

    fs::remove_dir_all(&root).unwrap();
}

/// A tick reads the check-in it judges, spends seconds typing the nudge, and only then touches the
/// file. An assignment armed in that window belongs to work the tick has not seen: deleting it
/// would leave the fresh assignment with no check-in at all, silently.
#[test]
fn a_check_in_armed_while_a_disarm_is_delivering_is_left_alone() {
    let root = workspace("re-read-disarm");
    write_worker(&root, "impl", WINDOW);
    let dir = worker_dir(&root, "impl");
    Checkin::armed(Duration::from_secs(300), 0, at(1_000))
        .write(&dir)
        .unwrap();
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    append_log(&root, "impl", DONE);

    let snapshot = worker_snapshot(&root).unwrap();
    let mut sink = RecordingSink::default();
    let checkins = read_checkins(&snapshot, &mut sink);
    let plan = memory.plan(&snapshot, &checkins, at(1_060));
    assert_eq!(
        plan.disarms.len(),
        1,
        "the report answers the armed check-in"
    );

    // The lead dispatches again while the tick is still delivering.
    let fresh = Checkin::armed(
        Duration::from_secs(300),
        (WINDOW.len() + DONE.len()) as u64,
        at(1_060),
    );
    fresh.write(&dir).unwrap();

    apply(&plan, &mut memory, &mut sink, &running());

    assert_eq!(
        Checkin::read(&dir).unwrap(),
        Some(fresh),
        "the newer assignment must keep its check-in"
    );
    assert!(
        sink.notes
            .iter()
            .any(|note| note.contains("left the check-in for impl alone")),
        "{sink:?}"
    );

    fs::remove_dir_all(&root).unwrap();
}

/// The same window, on the re-arm path: a fired check-in is rewritten at +3 minutes only if it is
/// still the one this tick planned over, or the newer assignment would be scheduled by the older
/// one's clock.
#[test]
fn a_check_in_armed_while_a_fired_nudge_is_delivering_is_not_overwritten() {
    let root = workspace("re-read-rearm");
    write_worker(&root, "impl", WINDOW);
    let dir = worker_dir(&root, "impl");
    Checkin::armed(Duration::from_secs(300), 0, at(1_000))
        .write(&dir)
        .unwrap();
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());

    let snapshot = worker_snapshot(&root).unwrap();
    let mut sink = RecordingSink::default();
    let checkins = read_checkins(&snapshot, &mut sink);
    let plan = memory.plan(&snapshot, &checkins, at(1_300));
    assert_eq!(plan.nudges.len(), 1, "the check-in is due");

    // Re-armed in the middle of the nudge, because the worker reported and was dispatched again.
    let fresh = Checkin::armed(Duration::from_secs(300), WINDOW.len() as u64, at(1_300));
    fresh.write(&dir).unwrap();

    apply(&plan, &mut memory, &mut sink, &running());

    assert_eq!(Checkin::read(&dir).unwrap(), Some(fresh), "{sink:?}");

    fs::remove_dir_all(&root).unwrap();
}

/// The lead's stdout and stderr belong to its TUI: a line the watcher printed would land in the
/// middle of the agent's own screen. The watcher is a thread inside that process, so "it never
/// writes to them" is a property of its source, and every path out of it goes through the sink —
/// whose diagnostics land in `watch.log` and whose only other act is a tmux `send_line`.
#[test]
fn the_watcher_has_no_stdout_or_stderr_of_its_own() {
    for (name, source) in [
        ("watch.rs", include_str!("../watch.rs")),
        ("checkin.rs", include_str!("checkin.rs")),
        ("decide.rs", include_str!("decide.rs")),
    ] {
        for macro_call in ["println!", "eprintln!", "print!", "dbg!"] {
            assert!(
                !source.contains(macro_call),
                "{name} must not use {macro_call}: the lead's streams are not the watcher's"
            );
        }
    }
}
