//! The watcher's decision: given what the workspace looks like now, what the thread has already
//! accounted for, and which check-ins are armed — what should be said to the lead.
//!
//! Pure apart from the bookkeeping of which workers it has seen before: no clock of its own (the
//! tick passes `now`), no filesystem, no tmux. `watch.rs` reads the state, asks here, and delivers
//! whatever comes back through the one edge it owns.
//!
//! Everything it decides is STATE, not an event. A nudge says where things stand and carries no
//! status-line content, so the same answer twice is harmless and there is no cursor, lock or
//! exactly-once rule anywhere in this module. Report detection keys on log LENGTH rather than the
//! state word, so `done:` → follow-up → `done:` is two reports and not a replay of one.

use std::collections::{BTreeMap, BTreeSet};

use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};

use crate::{wake::WakeKind, worker::WorkerSnapshot};

use super::checkin::Checkin;

/// What one tick should do, in the order the edge does it.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) nudges: Vec<Nudge>,
    /// Check-ins the worker has answered: the arm state goes, and a fresh assignment arms its own.
    pub(crate) disarms: Vec<Disarm>,
}

/// A check-in the worker answered, and the state it was read from.
///
/// The planned state travels with the decision because the tick re-reads the file before deleting
/// it: a tick spends seconds inside `send_line`, and an assignment armed in that window must not be
/// thrown away along with the one it planned over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Disarm {
    pub(crate) id: String,
    pub(crate) worker_dir: Utf8PathBuf,
    pub(crate) planned: Checkin,
}

/// One line to type into the lead's pane, and what to record once it has landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Nudge {
    pub(crate) id: String,
    pub(crate) worker_dir: Utf8PathBuf,
    pub(crate) text: String,
    pub(crate) commit: Commit,
}

/// The state a delivered nudge moves forward.
///
/// Nothing is recorded for a nudge that did not land, which is the whole retry mechanism: the next
/// tick sees the same log, reaches the same decision, and types it again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Commit {
    /// Everything up to this log length has been reported; the next report is a later line.
    Seen(u64),
    /// This check-in landed: `next` replaces `planned`, if `planned` is still what is on disk.
    Checkin { planned: Checkin, next: Checkin },
}

/// What the thread remembers between ticks.
///
/// In memory because the process is the lead's: there is no lock file, no recorded pid and no
/// second process to agree with. Nothing here is shared with `niles wait`, whose cursor stays the
/// only thing that consumes a status line.
#[derive(Debug, Default)]
pub(crate) struct WatchMemory {
    seen: BTreeMap<String, u64>,
}

impl WatchMemory {
    /// Memory for a watcher that is starting now.
    ///
    /// Everything already in a live worker's log is history: a `done:` written before this lead
    /// even launched must not nudge on every restart.
    pub(crate) fn at_start(snapshot: &[WorkerSnapshot]) -> Self {
        Self {
            seen: snapshot
                .iter()
                .map(|worker| (worker.id.clone(), worker.log_len))
                .collect(),
        }
    }

    /// Decides this tick's nudges and disarms.
    pub(crate) fn plan(
        &mut self,
        snapshot: &[WorkerSnapshot],
        checkins: &BTreeMap<String, Checkin>,
        now: DateTime<Utc>,
    ) -> Plan {
        let mut plan = Plan::default();
        let mut live = BTreeSet::new();

        for worker in snapshot {
            live.insert(worker.id.clone());
            let seen = self.seen.entry(worker.id.clone()).or_insert(0);
            // A log that shrank is a different log: the worker was closed and respawned under this
            // id, or its log was rewritten. The old offset describes nothing, so start over
            // instead of ignoring everything the new one says.
            if worker.log_len < *seen {
                *seen = 0;
            }
            let accounted = *seen;

            if let Some(checkin) = checkins.get(&worker.id) {
                match worker.last_actionable {
                    Some(wake) if checkin.answered_by(wake) => plan.disarms.push(Disarm {
                        id: worker.id.clone(),
                        worker_dir: worker.worker_dir.clone(),
                        planned: *checkin,
                    }),
                    _ if now >= checkin.deadline => plan.nudges.push(Nudge {
                        id: worker.id.clone(),
                        worker_dir: worker.worker_dir.clone(),
                        text: no_report_text(&worker.id, &checkin.elapsed_label()),
                        commit: Commit::Checkin {
                            planned: *checkin,
                            next: checkin.rearmed(now),
                        },
                    }),
                    _ => {}
                }
            }

            if worker.log_len > accounted
                && let Some(wake) = worker.last_actionable
                && wake.end > accounted
            {
                plan.nudges.push(Nudge {
                    id: worker.id.clone(),
                    worker_dir: worker.worker_dir.clone(),
                    text: report_text(&worker.id, wake.kind),
                    commit: Commit::Seen(worker.log_len),
                });
            }
        }

        // A worker that is gone takes its arm state with it: the check-in file lives in the
        // directory that was archived or removed.
        self.seen.retain(|id, _| live.contains(id));
        plan
    }

    /// Records a nudge that landed.
    pub(crate) fn commit(&mut self, nudge: &Nudge) {
        if let Commit::Seen(len) = nudge.commit {
            self.seen.insert(nudge.id.clone(), len);
        }
    }
}

/// `niles: impl reported (done) — check workers`
fn report_text(id: &str, kind: WakeKind) -> String {
    format!("niles: {id} reported ({kind}) — check workers")
}

/// `niles: no report from impl in 5m — check it`
fn no_report_text(id: &str, elapsed: &str) -> String {
    format!("niles: no report from {id} in {elapsed} — check it")
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use super::*;

    const WINDOW: &str = "working: launch\n";
    const DONE: &str = "done: shipped\n";

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(seconds, 0).unwrap()
    }

    fn worker(id: &str, log: &str) -> WorkerSnapshot {
        WorkerSnapshot::new(
            id.to_owned(),
            Utf8PathBuf::from_path_buf(PathBuf::from(format!("/w/{id}"))).unwrap(),
            Some(log.as_bytes().to_vec()),
        )
    }

    /// A check-in armed the way `spawn` arms one over `log`.
    fn armed_over(log: &str, delay_secs: u64, now: DateTime<Utc>) -> BTreeMap<String, Checkin> {
        BTreeMap::from([(
            "impl".to_owned(),
            Checkin::armed(
                Duration::from_secs(delay_secs),
                worker("impl", log).log_len,
                now,
            ),
        )])
    }

    fn reports(plan: &Plan) -> Vec<&str> {
        plan.nudges
            .iter()
            .filter(|nudge| matches!(nudge.commit, Commit::Seen(_)))
            .map(|nudge| nudge.text.as_str())
            .collect()
    }

    #[test]
    fn a_report_right_after_dispatch_nudges_once() {
        let now = at(1_000);
        let checkins = armed_over(WINDOW, 300, now);
        let mut memory = WatchMemory::at_start(&[worker("impl", WINDOW)]);
        let dispatched = || worker("impl", &format!("{WINDOW}{DONE}"));

        let plan = memory.plan(&[dispatched()], &checkins, at(1060));
        assert_eq!(
            reports(&plan),
            vec!["niles: impl reported (done) — check workers"]
        );
        // The worker answered the assignment, so the check-in goes with it.
        assert_eq!(plan.disarms.len(), 1);
        assert_eq!(plan.disarms[0].id, "impl");

        // Nothing has changed since: the same state is not nudged again.
        memory.commit(&plan.nudges[0]);
        let quiet = memory.plan(&[dispatched()], &BTreeMap::new(), at(1061));
        assert!(quiet.nudges.is_empty(), "{quiet:?}");
    }

    #[test]
    fn done_send_done_nudges_twice() {
        let mut memory = WatchMemory::at_start(&[worker("impl", WINDOW)]);
        let first = format!("{WINDOW}{DONE}");
        let second = format!("{first}done: shipped again\n");

        let plan = memory.plan(&[worker("impl", &first)], &BTreeMap::new(), at(1030));
        assert_eq!(
            reports(&plan),
            vec!["niles: impl reported (done) — check workers"]
        );
        memory.commit(&plan.nudges[0]);

        // `send` appends nothing to the log; the second report is later text of the same shape,
        // and the length is what tells it apart from a replay.
        let plan = memory.plan(&[worker("impl", &second)], &BTreeMap::new(), at(1060));
        assert_eq!(
            reports(&plan),
            vec!["niles: impl reported (done) — check workers"]
        );
    }

    #[test]
    fn a_log_that_only_grew_with_working_lines_nudges_nobody() {
        let mut memory = WatchMemory::at_start(&[worker("impl", WINDOW)]);
        let busy = worker("impl", &format!("{WINDOW}working: still going\n"));

        let plan = memory.plan(&[busy], &BTreeMap::new(), at(1300));

        assert!(plan.nudges.is_empty(), "{plan:?}");
    }

    #[test]
    fn history_present_at_thread_start_never_nudges() {
        let mut memory = WatchMemory::at_start(&[worker("impl", &format!("{WINDOW}{DONE}"))]);

        let plan = memory.plan(
            &[worker("impl", &format!("{WINDOW}{DONE}"))],
            &BTreeMap::new(),
            at(1_000),
        );

        assert!(plan.nudges.is_empty(), "{plan:?}");
    }

    /// The log length is the identity of what has been said: a log that shrank is a new log.
    #[test]
    fn a_replaced_log_starts_over() {
        let mut memory = WatchMemory::at_start(&[worker("impl", "done: a much longer report\n")]);

        let plan = memory.plan(&[worker("impl", DONE)], &BTreeMap::new(), at(1_000));

        assert_eq!(
            reports(&plan),
            vec!["niles: impl reported (done) — check workers"]
        );
    }

    #[test]
    fn check_ins_fire_at_five_eight_and_eleven_minutes_with_no_report() {
        let now = at(1_000);
        let mut memory = WatchMemory::at_start(&[worker("impl", WINDOW)]);
        let mut armed = armed_over(WINDOW, 300, now);
        let busy = || worker("impl", WINDOW);

        // Not due yet.
        let early = memory.plan(&[busy()], &armed, at(1299));
        assert!(early.nudges.is_empty(), "{early:?}");

        for (offset, minutes) in [(300, 5), (480, 8), (660, 11)] {
            let plan = memory.plan(&[busy()], &armed, at(1_000 + offset));
            assert_eq!(
                plan.nudges
                    .iter()
                    .map(|nudge| nudge.text.as_str())
                    .collect::<Vec<_>>(),
                vec![format!(
                    "niles: no report from impl in {minutes}m — check it"
                )],
                "at {offset}s"
            );
            assert!(plan.disarms.is_empty());

            // The re-arm only happens because the nudge landed; it is what stops the next tick
            // from firing the same check-in again.
            let nudge = plan.nudges[0].clone();
            memory.commit(&nudge);
            let Commit::Checkin { next, .. } = nudge.commit else {
                panic!("a check-in nudge must carry its re-arm");
            };
            assert_eq!(next.elapsed_label(), format!("{}m", minutes + 3));
            armed = BTreeMap::from([("impl".to_owned(), next)]);
        }
    }

    #[test]
    fn a_failed_check_in_nudge_is_not_recorded_so_the_next_tick_fires_it_again() {
        let now = at(1_000);
        let checkins = armed_over(WINDOW, 300, now);
        let mut memory = WatchMemory::at_start(&[worker("impl", WINDOW)]);
        let busy = || worker("impl", WINDOW);

        let first = memory.plan(&[busy()], &checkins, at(1300));
        // No commit: the send failed, so nothing moved.
        let retry = memory.plan(&[busy()], &checkins, at(1301));

        assert_eq!(first.nudges.len(), 1);
        assert_eq!(
            first.nudges[0].text, retry.nudges[0].text,
            "the same check-in comes round again rather than being lost"
        );
        // Only the re-arm deadline moved: it is computed from the tick that is trying, since the
        // nudge it belongs to has not landed yet.
        assert_ne!(first.nudges[0].commit, retry.nudges[0].commit);
    }

    #[test]
    fn an_actionable_line_past_the_armed_length_disarms_and_an_older_one_does_not() {
        let now = at(1_000);
        let checkins = armed_over(WINDOW, 300, now);
        let mut memory = WatchMemory::at_start(&[worker("impl", WINDOW)]);

        // A `done:` that was already in the log when the lead armed the check-in is history: it
        // answers an earlier assignment, not this one.
        let stale = worker("impl", &format!("{WINDOW}{DONE}"));
        let mut stale_checkins = checkins.clone();
        stale_checkins.insert(
            "impl".to_owned(),
            Checkin::armed(Duration::from_secs(300), stale.log_len + 10, now),
        );
        let plan = memory.plan(&[stale], &stale_checkins, at(1300));
        assert!(plan.disarms.is_empty(), "{plan:?}");
        memory.commit(&plan.nudges[0]);

        // A report that lands after arming does answer it.
        let plan = memory.plan(
            &[worker("impl", &format!("{WINDOW}{DONE}done: answered\n"))],
            &checkins,
            at(1300),
        );
        assert_eq!(plan.disarms.len(), 1, "{plan:?}");
    }

    #[test]
    fn a_worker_that_is_gone_is_forgotten() {
        let now = at(1_000);
        let mut memory = WatchMemory::at_start(&[worker("impl", WINDOW)]);
        let reported = worker("impl", &format!("{WINDOW}{DONE}"));

        // Gone for a tick: nothing to say about it.
        let gone = memory.plan(&[], &BTreeMap::new(), now);
        assert!(gone.nudges.is_empty(), "{gone:?}");

        // A worker id that appears again is a new incarnation — it was closed and respawned — so
        // its log is read from the start rather than against the dead worker's length.
        let plan = memory.plan(&[reported], &BTreeMap::new(), at(1001));
        assert_eq!(
            reports(&plan),
            vec!["niles: impl reported (done) — check workers"]
        );
    }
}
