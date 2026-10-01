use std::{fs, sync::atomic::AtomicBool, time::Duration};

use anyhow::{Result, bail};
use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};

use crate::{
    test_support::{at, temp_test_path},
    tmux::TmuxTarget,
    worker::worker_snapshot,
};

use super::{
    Sink, WatchMemory, apply, arm_checkin,
    cadence::{Cadence, Recheck},
    checkin::Checkin,
    quiet, read_checkins, tick,
    trust::claude_prompt,
};

fn running() -> AtomicBool {
    AtomicBool::new(false)
}

fn armed_checkin(delay_secs: u64, armed_len: u64, now: DateTime<Utc>) -> Checkin {
    Checkin::armed(
        Duration::from_secs(delay_secs),
        Recheck::Backoff,
        armed_len,
        now,
    )
}

#[derive(Debug, Default)]
struct RecordingSink {
    attempts: Vec<String>,
    sent: Vec<String>,
    captures: Vec<String>,
    notes: Vec<String>,
    fail: bool,
    capture_failure: bool,
    screen: String,
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

    fn capture_visible(&mut self, target: &TmuxTarget) -> Result<String> {
        self.captures.push(target.as_str().to_owned());
        if self.capture_failure {
            bail!("stub worker pane vanished");
        }
        Ok(self.screen.clone())
    }

    fn note(&mut self, line: &str) {
        self.notes.push(line.to_owned());
    }
}

pub(super) fn workspace(label: &str) -> Utf8PathBuf {
    let path = temp_test_path(label);
    fs::create_dir_all(&path).unwrap();
    path
}

pub(super) fn worker_dir(workspace: &Utf8PathBuf, id: &str) -> Utf8PathBuf {
    workspace.join(".niles/worker").join(id)
}

fn write_worker(workspace: &Utf8PathBuf, id: &str, log: &str) {
    write_meta(workspace, id, None);
    let dir = worker_dir(workspace, id);
    fs::write(dir.join("status.log"), log).unwrap();
}

fn write_meta(workspace: &Utf8PathBuf, id: &str, created_at: Option<DateTime<Utc>>) {
    let dir = worker_dir(workspace, id);
    fs::create_dir_all(&dir).unwrap();
    let (created_at, session) = match created_at {
        Some(time) => (
            format!(",\"created_at\":\"{}\"", time.to_rfc3339()),
            "ambient",
        ),
        None => (String::new(), "niles-test"),
    };
    fs::write(
        dir.join("meta.json"),
        format!(
            "{{\"niles_schema\":2,\"id\":\"{id}\",\"agent\":\"claude\"{created_at},\
             \"project\":\"{workspace}\",\"window\":\"{session}:niles-{id}\",\
             \"brief\":\"{workspace}/brief.md\",\
             \"launch\":\"{workspace}/launch.sh\"}}"
        ),
    )
    .unwrap();
}

pub(super) fn write_starting_worker(
    workspace: &Utf8PathBuf,
    id: &str,
    log: &str,
    now: DateTime<Utc>,
) {
    write_meta(workspace, id, Some(now));
    fs::write(worker_dir(workspace, id).join("status.log"), log).unwrap();
}

fn append_log(workspace: &Utf8PathBuf, id: &str, line: &str) {
    let path = worker_dir(workspace, id).join("status.log");
    let body = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{body}{line}")).unwrap();
}

fn status(workspace: &Utf8PathBuf, id: &str) -> String {
    fs::read_to_string(worker_dir(workspace, id).join("status.log")).unwrap()
}

const WINDOW: &str = "working: launch\n";
const DONE: &str = "done: shipped\n";

#[test]
fn workspace_trust_follows_the_worker_and_checkin_lifecycle() {
    let root = workspace("trust-lifecycle");
    let now = at(1_000);
    write_starting_worker(&root, "armed", "", now);
    write_starting_worker(&root, "plain", "", now);
    write_starting_worker(&root, "expired", "", at(969));
    write_starting_worker(&root, "reporting", WINDOW, now);
    let armed_dir = worker_dir(&root, "armed");
    let armed = armed_checkin(90, 0, now);
    armed.write(&armed_dir).unwrap();
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    let mut sink = RecordingSink {
        capture_failure: true,
        ..RecordingSink::default()
    };
    tick(&root, now, &mut memory, &mut sink, &running());
    assert_eq!(sink.captures.len(), 2);
    assert_eq!(sink.notes.len(), 2);
    assert_eq!(status(&root, "armed"), "");

    sink = RecordingSink {
        screen: claude_prompt(&root),
        ..RecordingSink::default()
    };
    tick(&root, now, &mut memory, &mut sink, &running());
    let blocked = |id| {
        format!(
            "blocked: workspace trust confirmation needs operator action in worker pane \
             ambient:niles-{id}\n"
        )
    };
    assert!(sink.sent.is_empty());
    assert_eq!(sink.captures.len(), 2);
    for (id, expected) in [
        ("armed", blocked("armed")),
        ("plain", blocked("plain")),
        ("expired", String::new()),
        ("reporting", WINDOW.to_owned()),
    ] {
        assert_eq!(status(&root, id), expected);
    }
    assert!(!armed_dir.join("status.cursor").exists());
    assert!(!worker_dir(&root, "plain").join("checkin").exists());
    let mut preserved = armed;
    preserved.armed_len = blocked("armed").len() as u64;
    assert_eq!(Checkin::read(&armed_dir).unwrap(), Some(preserved));

    sink = RecordingSink::default();
    tick(&root, at(1_001), &mut memory, &mut sink, &running());
    assert_eq!(sink.sent.len(), 2);
    assert_eq!(
        sink.sent[0],
        "niles: armed reported (blocked) — check workers"
    );
    assert!(sink.captures.is_empty());

    let mut restarted = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    sink = RecordingSink::default();
    tick(&root, at(1_002), &mut restarted, &mut sink, &running());
    assert!(sink.captures.is_empty());
    assert!(sink.sent.is_empty());
    assert_eq!(status(&root, "armed").lines().count(), 1);

    sink = RecordingSink::default();
    tick(&root, at(1_090), &mut memory, &mut sink, &running());
    assert_eq!(sink.sent, ["niles: no report from armed in 90s — check it"]);
    assert!(Checkin::read(&armed_dir).unwrap().is_some());

    append_log(&root, "armed", DONE);
    sink = RecordingSink::default();
    tick(&root, at(1_091), &mut memory, &mut sink, &running());
    assert_eq!(sink.sent, ["niles: armed reported (done) — check workers"]);
    assert_eq!(Checkin::read(&armed_dir).unwrap(), None);

    fs::remove_dir_all(&root).unwrap();
}

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
    armed_checkin(300, WINDOW.len() as u64, now)
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
    armed_checkin(300, WINDOW.len() as u64, now)
        .write(&dir)
        .unwrap();
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());

    let mut sink = RecordingSink::default();
    tick(&root, at(1300), &mut memory, &mut sink, &running());

    assert_eq!(
        sink.sent,
        vec!["niles: no report from impl in 5m — check it"]
    );
    let rearmed = Checkin::read(&dir).unwrap().unwrap();
    assert_eq!(rearmed.deadline, at(1_900));
    assert_eq!(rearmed.delay, 600);
    assert_eq!(rearmed.elapsed_label(), "15m");

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
    armed_checkin(300, WINDOW.len() as u64, now)
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

    let armed = arm_checkin(
        &dir,
        Cadence {
            delay: Some(Duration::from_secs(90)),
            recheck: Recheck::Backoff,
        },
        baseline,
        at(1_000),
    )
    .unwrap();

    assert_eq!(armed, Some(Duration::from_secs(90)));
    let checkin = Checkin::read(&dir).unwrap().unwrap();
    assert_eq!(checkin.armed_len, baseline);
    assert_eq!(checkin.deadline, at(1_090));

    assert_eq!(
        arm_checkin(
            &dir,
            Cadence {
                delay: None,
                recheck: Recheck::Backoff,
            },
            baseline,
            at(1_000)
        )
        .unwrap(),
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

    let mut later = RecordingSink::default();
    tick(&root, at(1_001), &mut memory, &mut later, &running());
    assert_eq!(
        later.sent,
        vec!["niles: impl reported (done) — check workers"]
    );

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_check_in_armed_while_a_disarm_is_delivering_is_left_alone() {
    let root = workspace("re-read-disarm");
    write_worker(&root, "impl", WINDOW);
    let dir = worker_dir(&root, "impl");
    armed_checkin(300, 0, at(1_000)).write(&dir).unwrap();
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
    let fresh = armed_checkin(300, (WINDOW.len() + DONE.len()) as u64, at(1_060));
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

#[test]
fn a_check_in_armed_while_a_fired_nudge_is_delivering_is_not_overwritten() {
    let root = workspace("re-read-rearm");
    write_worker(&root, "impl", WINDOW);
    let dir = worker_dir(&root, "impl");
    armed_checkin(300, 0, at(1_000)).write(&dir).unwrap();
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());

    let snapshot = worker_snapshot(&root).unwrap();
    let mut sink = RecordingSink::default();
    let checkins = read_checkins(&snapshot, &mut sink);
    let plan = memory.plan(&snapshot, &checkins, at(1_300));
    assert_eq!(plan.nudges.len(), 1, "the check-in is due");

    // Re-armed in the middle of the nudge, because the worker reported and was dispatched again.
    let fresh = armed_checkin(300, WINDOW.len() as u64, at(1_300));
    fresh.write(&dir).unwrap();

    apply(&plan, &mut memory, &mut sink, &running());

    assert_eq!(Checkin::read(&dir).unwrap(), Some(fresh), "{sink:?}");

    fs::remove_dir_all(&root).unwrap();
}
