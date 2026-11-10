use super::cadence::{Cadence, Delay, Recheck};
use super::checkin::Checkin;
use super::trust::claude_prompt;
use super::{Sink, WatchMemory, apply, arm_checkin, quiet, read_checkins, tick};
use crate::store::{self, atomic_write};
use crate::telemetry::{ResolvedSession, SessionState};
use crate::test_support::{
    self, TempDir, assert_err_contains, at, temp_dir, temp_test_path, write_worker_meta,
};
use crate::tmux::TmuxTarget;
use crate::wait::cursor;
use crate::worker::{self, WorkerSnapshot, worker_snapshot};
use anyhow::{Result, bail};
use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};
use std::fs;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

mod batch;
mod hold;
mod refusal;
mod taken;

fn running() -> AtomicBool {
    AtomicBool::new(false)
}

fn armed_checkin(delay_secs: u64, armed_len: u64, now: DateTime<Utc>) -> Checkin {
    Checkin::armed(Delay::seconds(delay_secs), Recheck::Backoff, armed_len, now).unwrap()
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
    replace_status: Option<Utf8PathBuf>,
    quiet_during_capture: Option<Utf8PathBuf>,
    typed: bool,
    activity: Option<SessionState>,
}

impl Sink for RecordingSink {
    fn activity(&mut self, worker: &WorkerSnapshot, now: DateTime<Utc>) -> Result<ResolvedSession> {
        match self.activity {
            Some(state) => Ok(ResolvedSession {
                state,
                blocker: None,
                usage: None,
            }),
            None => worker::worker_session(worker, now),
        }
    }
    fn has_draft(&mut self) -> bool {
        self.typed
    }
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
        if let Some(dir) = &self.quiet_during_capture {
            let lock = cursor::open_lock_file(&dir.join("checkin.lock"))?;
            lock.try_lock()?;
            Checkin::disarm(dir)?;
        }
        if let Some(path) = &self.replace_status {
            fs::remove_file(path)?;
            atomic_write(path, "")?;
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

pub(super) fn worker_dir(workspace: &camino::Utf8Path, id: &str) -> Utf8PathBuf {
    store::workers_dir(workspace).unwrap().join(id)
}

fn write_worker(workspace: &camino::Utf8Path, id: &str, log: &str) {
    write_meta(workspace, id, at(0), "niles-test");
    let dir = worker_dir(workspace, id);
    atomic_write(dir.join("status.log"), log).unwrap();
}

fn write_meta(workspace: &camino::Utf8Path, id: &str, created_at: DateTime<Utc>, session: &str) {
    write_worker_meta(
        workspace,
        id,
        &format!("{session}:niles-{id}"),
        created_at,
        None,
    );
}

pub(super) fn write_starting_worker(
    workspace: &camino::Utf8Path,
    id: &str,
    log: &str,
    now: DateTime<Utc>,
) {
    write_meta(workspace, id, now, "ambient");
    atomic_write(worker_dir(workspace, id).join("status.log"), log).unwrap();
}

fn append_log(workspace: &camino::Utf8Path, id: &str, line: &str) {
    let path = worker_dir(workspace, id).join("status.log");
    let body = fs::read_to_string(&path).unwrap();
    atomic_write(&path, format!("{body}{line}")).unwrap();
}

fn status(workspace: &camino::Utf8Path, id: &str) -> String {
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
    tick(&root, now, &mut memory, &mut sink, &running()).unwrap();
    assert_eq!(sink.captures.len(), 2);
    assert_eq!(sink.notes.len(), 2);
    assert_eq!(status(&root, "armed"), "");

    sink = RecordingSink {
        screen: claude_prompt(&root),
        ..RecordingSink::default()
    };
    tick(&root, now, &mut memory, &mut sink, &running()).unwrap();
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
    tick(&root, at(1_001), &mut memory, &mut sink, &running()).unwrap();
    assert_eq!(
        sink.sent,
        ["niles: armed reported (blocked), plain reported (blocked) — check workers"]
    );
    assert!(sink.captures.is_empty());

    let mut restarted = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    sink = RecordingSink::default();
    tick(&root, at(1_002), &mut restarted, &mut sink, &running()).unwrap();
    assert!(sink.captures.is_empty());
    assert!(sink.sent.is_empty());
    assert_eq!(status(&root, "armed").lines().count(), 1);

    sink = RecordingSink::default();
    tick(&root, at(1_090), &mut memory, &mut sink, &running()).unwrap();
    assert_eq!(sink.sent, ["niles: no report from armed — check workers"]);
    assert!(Checkin::read(&armed_dir).unwrap().is_some());

    append_log(&root, "armed", DONE);
    sink = RecordingSink::default();
    tick(&root, at(1_091), &mut memory, &mut sink, &running()).unwrap();
    assert_eq!(sink.sent, [reported("armed", "done")]);
    assert_eq!(Checkin::read(&armed_dir).unwrap(), None);

    write_starting_worker(&root, "impl", "", now);
    let dir = worker_dir(&root, "impl");
    armed.write(&dir).unwrap();
    let path = dir.join("status.log");
    sink = RecordingSink {
        quiet_during_capture: Some(dir.clone()),
        replace_status: Some(path.clone()),
        screen: claude_prompt(&root),
        ..RecordingSink::default()
    };
    super::trust::inspect_starting_workers(&worker_snapshot(&root).unwrap(), now, &mut sink);
    assert!(sink.notes.is_empty(), "{:?}", sink.notes);
    assert!(sink.sent.is_empty());
    assert_eq!(fs::read_to_string(&path).unwrap(), "");
    assert_eq!(Checkin::read(&dir).unwrap(), None);
    test_support::remove_workspace(&root).unwrap();
}

fn due_checkin() -> Checkin {
    armed_checkin(300, WINDOW.len() as u64, at(1_000))
}

fn one_worker(label: &str, checkin: Option<Checkin>) -> (TempDir, WatchMemory) {
    let root = temp_dir(label);
    write_worker(&root, "impl", WINDOW);
    if let Some(checkin) = checkin {
        checkin.write(&worker_dir(&root, "impl")).unwrap();
    }
    let memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    (root, memory)
}

pub(super) fn reported(id: &str, kind: &str) -> String {
    format!("niles: {id} reported ({kind}) — check workers")
}

#[test]
fn failed_or_stopping_ticks_leave_reports_for_the_next_tick() {
    for stop in [false, true] {
        let (root, mut memory) = one_worker("retry-or-stop", None);
        append_log(&root, "impl", DONE);
        let mut sink = RecordingSink {
            fail: !stop,
            ..RecordingSink::default()
        };
        tick(
            &root,
            at(1_000),
            &mut memory,
            &mut sink,
            &AtomicBool::new(stop),
        )
        .unwrap();
        assert_eq!(sink.attempts.len(), usize::from(!stop));
        assert!(sink.sent.is_empty(), "{sink:?}");
        sink.fail = false;
        tick(&root, at(1_001), &mut memory, &mut sink, &running()).unwrap();
        assert_eq!(sink.sent, [reported("impl", "done")]);
        let attempts = sink.attempts.len();
        tick(&root, at(1_002), &mut memory, &mut sink, &running()).unwrap();
        assert_eq!(sink.attempts.len(), attempts, "{sink:?}");
        test_support::remove_workspace(&root).unwrap();
    }
}

#[test]
fn ticks_persist_checkins_only_after_successful_delivery() {
    let armed = due_checkin();
    for (label, report, now, activity, fail, expected, sent) in [
        (
            "disarm",
            DONE,
            1_060,
            None,
            false,
            None,
            reported("impl", "done"),
        ),
        (
            "rearm",
            "",
            1_300,
            Some(SessionState::Starting),
            false,
            Some(Checkin {
                deadline: at(1_900),
                delay: Delay::seconds(600),
                ..armed
            }),
            "niles: no report from impl — check workers".into(),
        ),
        (
            "idle-disarm",
            "",
            1_300,
            Some(SessionState::Waiting),
            false,
            None,
            "niles: impl stopped without a report — check workers".into(),
        ),
        (
            "rearm-failed",
            "",
            1_300,
            Some(SessionState::Starting),
            true,
            Some(armed),
            String::new(),
        ),
        (
            "gone-report",
            "gone: window exited\n",
            1_300,
            None,
            false,
            None,
            reported("impl", "gone"),
        ),
    ] {
        let (root, mut memory) = one_worker(label, Some(armed));
        let dir = worker_dir(&root, "impl");
        append_log(&root, "impl", report);
        let mut sink = RecordingSink {
            activity,
            fail,
            ..RecordingSink::default()
        };
        tick(&root, at(now), &mut memory, &mut sink, &running()).unwrap();
        assert_eq!(Checkin::read(&dir).unwrap(), expected, "{label}: {sink:?}");
        assert_eq!(dir.join("checkin").exists(), expected.is_some(), "{label}");
        assert_eq!(sink.sent, if fail { vec![] } else { vec![sent] }, "{label}");
        if matches!(label, "rearm" | "gone-report") {
            tick(&root, at(1_301), &mut memory, &mut sink, &running()).unwrap();
            assert_eq!(sink.attempts.len(), 1, "{label}: {sink:?}");
        }
        test_support::remove_workspace(&root).unwrap();
    }
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
            delay: Some(Delay::seconds(90)),
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

    test_support::remove_workspace(&root).unwrap();
}

#[test]
fn quiet_is_a_lead_command_and_needs_a_worker_that_exists() {
    assert_err_contains(quiet("no-such-worker"), "unknown worker id");
}

#[test]
fn watcher_start_follows_the_recorded_lead_pane() {
    for (pane, required, forbidden) in [
        (None, vec!["recorded no lead pane"], vec!["watcher started"]),
        (
            Some("%7"),
            vec![
                "watcher started: pane=%7",
                "watcher stopped: the foreground agent exited",
            ],
            vec![],
        ),
    ] {
        let session = temp_dir("pane-session");
        let root = workspace("pane-workspace");
        write_worker(&root, "impl", &format!("{WINDOW}{DONE}"));
        drop(super::start(&session, &root, pane, None));
        let log = fs::read_to_string(session.join("watch.log")).unwrap();
        for phrase in required {
            assert!(log.contains(phrase), "{log}");
        }
        for phrase in forbidden {
            assert!(!log.contains(phrase), "{log}");
        }
        test_support::remove_workspace(&root).unwrap();
    }
}

#[test]
fn a_checkin_rearmed_during_delivery_is_left_alone() {
    for (report, plan_at, fresh_len) in [
        (DONE, 1_060, WINDOW.len() + DONE.len()),
        ("", 1_300, WINDOW.len()),
    ] {
        let (root, mut memory) = one_worker("re-read", Some(armed_checkin(300, 0, at(1_000))));
        let dir = worker_dir(&root, "impl");
        append_log(&root, "impl", report);
        let snapshot = worker_snapshot(&root).unwrap();
        let mut sink = RecordingSink::default();
        let checkins = read_checkins(&snapshot, &mut sink);
        let plan = memory.plan(&snapshot, &checkins, at(plan_at)).unwrap();
        if report.is_empty() {
            assert_eq!(plan.nudges.len(), 1, "the check-in is due");
        } else {
            assert_eq!(
                plan.disarms.len(),
                1,
                "the report answers the armed check-in"
            );
        }
        // A new dispatch lands while the old tick is delivering.
        let fresh = armed_checkin(300, fresh_len as u64, at(plan_at));
        fresh.write(&dir).unwrap();
        apply(&plan, &mut memory, &mut sink, &running());
        assert_eq!(Checkin::read(&dir).unwrap(), Some(fresh), "{sink:?}");
        if !report.is_empty() {
            assert!(
                sink.notes
                    .iter()
                    .any(|note| note.contains("left the check-in for impl alone")),
                "{sink:?}"
            );
        }
        test_support::remove_workspace(&root).unwrap();
    }
}
