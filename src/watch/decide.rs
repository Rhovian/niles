//! The watcher's decision: given what the workspace looks like now, what the thread has already
//! accounted for, and which check-ins are armed — what should be said to the lead.
//!
//! Pure apart from the bookkeeping of which workers it has seen before: no clock of its own (the
//! tick passes `now`), no filesystem, no tmux. `watch.rs` reads the state, asks here, and delivers
//! whatever comes back through the one edge it owns.

use super::checkin::Checkin;
use super::digest::Item;
use crate::telemetry::SessionState;
use crate::worker::WorkerSnapshot;
use camino::Utf8PathBuf;
use chrono::{DateTime, TimeDelta, Utc};
use std::collections::{BTreeMap, BTreeSet};

const MAX_NUDGE_HOLD: TimeDelta = TimeDelta::minutes(5);

/// What one tick should do, in the order the edge does it.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) nudges: Vec<Item>,
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

/// What the thread remembers between ticks.
///
/// In memory because the process is the lead's: there is no lock file, no recorded pid and no
/// second process to agree with. `niles wait`'s cursor stays the only thing that consumes a status
/// line; this only reads it, so a line the lead already took from a wait is not reported again.
#[derive(Debug, Default)]
pub(crate) struct WatchMemory {
    seen: BTreeMap<String, u64>,
    held_since: Option<DateTime<Utc>>,
    pub(super) activity: BTreeMap<String, (DateTime<Utc>, Option<SessionState>)>,
}

impl WatchMemory {
    pub(crate) fn hold_nudge(&mut self, has_draft: bool, now: DateTime<Utc>) -> bool {
        if !has_draft {
            self.held_since = None;
            return false;
        }
        let since = *self.held_since.get_or_insert(now);
        now.signed_duration_since(since) < MAX_NUDGE_HOLD
    }
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
            ..Self::default()
        }
    }

    /// Decides this tick's nudges and disarms.
    pub(crate) fn plan(
        &mut self,
        snapshot: &[WorkerSnapshot],
        checkins: &BTreeMap<String, Checkin>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Plan> {
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
            // A line a wait has handed the lead, or is about to, is already the lead's: a nudge
            // for it would only send the lead to look again at nothing new.
            let taken = if worker.wait_running {
                worker.log_len
            } else {
                worker.delivered()
            };
            let accounted = (*seen).max(taken);

            if let Some(checkin) = checkins.get(&worker.id) {
                if worker
                    .last_actionable
                    .is_some_and(|wake| checkin.answered_by(wake))
                {
                    plan.disarms.push(Disarm {
                        id: worker.id.clone(),
                        worker_dir: worker.worker_dir.clone(),
                        planned: *checkin,
                    });
                } else if now >= checkin.deadline {
                    let state = self
                        .activity
                        .get(&worker.id)
                        .filter(|(deadline, _)| *deadline == checkin.deadline)
                        .and_then(|(_, state)| *state);
                    plan.nudges.push(Item::Checkin {
                        id: worker.id.clone(),
                        worker_dir: worker.worker_dir.clone(),
                        state,
                        planned: *checkin,
                        next: (state != Some(SessionState::Waiting))
                            .then(|| checkin.rearmed(now))
                            .transpose()?,
                    });
                }
            }

            if worker.log_len > accounted
                && let Some(wake) = worker.last_actionable
                && wake.end > accounted
            {
                plan.nudges.push(Item::Reported {
                    id: worker.id.clone(),
                    kind: wake.kind,
                    log_len: worker.log_len,
                });
            }
        }

        // A worker that is gone takes its arm state with it: the check-in file lives in the
        // directory that was archived or removed.
        self.seen.retain(|id, _| live.contains(id));
        self.activity.retain(|id, _| live.contains(id));
        Ok(plan)
    }

    /// Records a nudge that landed.
    pub(crate) fn commit(&mut self, nudge: &Item) {
        if let Item::Reported { id, log_len, .. } = nudge {
            self.seen.insert(id.clone(), *log_len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{super::cadence::Recheck, *};
    use crate::test_support::only;
    use crate::{
        test_support::at,
        watch::{cadence::Delay, tests::reported},
    };
    use std::path::PathBuf;

    const WINDOW: &str = "working: launch\n";

    const DONE: &str = "done: shipped\n";

    fn worker(id: &str, log: &str) -> WorkerSnapshot {
        WorkerSnapshot::new(
            id.to_owned(),
            Utf8PathBuf::from_path_buf(PathBuf::from(format!("/w/{id}"))).unwrap(),
            Some(log.as_bytes().to_vec()),
        )
    }

    fn fresh() -> WatchMemory {
        WatchMemory::at_start(&[worker("impl", WINDOW)])
    }

    /// A check-in armed the way `spawn` arms one over `log`: the delay asked for, doubling after.
    fn armed_over(log: &str, delay_secs: u64, now: DateTime<Utc>) -> BTreeMap<String, Checkin> {
        BTreeMap::from([(
            "impl".to_owned(),
            Checkin::armed(
                Delay::seconds(delay_secs),
                Recheck::Backoff,
                worker("impl", log).log_len,
                now,
            )
            .unwrap(),
        )])
    }

    fn reports(plan: &Plan) -> Vec<String> {
        plan.nudges
            .iter()
            .filter(|nudge| matches!(nudge, Item::Reported { .. }))
            .map(|nudge| nudge.text())
            .collect()
    }

    #[test]
    fn stage_does_not_answer_checkin_or_nudge() {
        let now = at(1_000);
        let checkins = armed_over(WINDOW, 300, now);
        let mut memory = fresh();
        let plan = memory
            .plan(
                &[worker("impl", &format!("{WINDOW}stage: ready\n"))],
                &checkins,
                at(1060),
            )
            .unwrap();
        assert!(plan.nudges.is_empty());
        assert!(plan.disarms.is_empty());
    }

    #[test]
    fn reports_disarm_then_only_new_report_text_nudges() {
        let checkins = armed_over(WINDOW, 300, at(1000));
        let mut memory = fresh();
        let first = format!("{WINDOW}{DONE}");
        let plan = memory
            .plan(&[worker("impl", &first)], &checkins, at(1060))
            .unwrap();
        assert_eq!(reports(&plan), vec![reported("impl", "done")]);
        assert_eq!(only(&plan.disarms).id, "impl");
        memory.commit(only(&plan.nudges));
        assert!(
            memory
                .plan(&[worker("impl", &first)], &BTreeMap::new(), at(1061))
                .unwrap()
                .nudges
                .is_empty()
        );
        let second = format!("{first}done: shipped again\n");
        let plan = memory
            .plan(&[worker("impl", &second)], &BTreeMap::new(), at(1062))
            .unwrap();
        assert_eq!(reports(&plan), vec![reported("impl", "done")]);
    }

    #[test]
    fn log_history_growth_and_replacement_control_reports() {
        for (start_log, next_log, expected) in [
            (
                WINDOW.to_owned(),
                format!("{WINDOW}working: still going\n"),
                vec![],
            ),
            (format!("{WINDOW}{DONE}"), format!("{WINDOW}{DONE}"), vec![]),
            (
                "done: a much longer report\n".into(),
                DONE.into(),
                vec![reported("impl", "done")],
            ),
        ] {
            let mut memory = WatchMemory::at_start(&[worker("impl", &start_log)]);
            let plan = memory
                .plan(&[worker("impl", &next_log)], &BTreeMap::new(), at(1_000))
                .unwrap();
            assert_eq!(reports(&plan), expected, "{start_log:?} -> {next_log:?}");
            if expected.is_empty() {
                assert!(plan.nudges.is_empty(), "{plan:?}");
            }
        }
    }

    #[test]
    fn check_ins_fire_at_five_fifteen_and_thirty_five_minutes_with_no_report() {
        let now = at(1_000);
        let mut memory = fresh();
        let mut armed = armed_over(WINDOW, 300, now);
        let busy = || worker("impl", WINDOW);

        let early = memory.plan(&[busy()], &armed, at(1299)).unwrap();
        assert!(early.nudges.is_empty(), "{early:?}");

        for (state, text, rearm) in [
            (
                Some(SessionState::Waiting),
                "niles: impl stopped without a report — check workers",
                false,
            ),
            (
                Some(SessionState::Unknown),
                "niles: no report from impl — check workers",
                true,
            ),
            (None, "niles: no report from impl — check workers", true),
        ] {
            memory.activity.insert(
                "impl".to_owned(),
                (armed.get("impl").unwrap().deadline, state),
            );
            let idle = memory.plan(&[busy()], &armed, at(1300)).unwrap();
            let nudge = only(&idle.nudges);
            assert_eq!(nudge.text(), text);
            assert!(matches!(nudge, Item::Checkin { next, .. } if next.is_some() == rearm));
        }

        memory.activity.clear();
        // The gap doubles after each fire: a silence of 5m, then 15m, then 35m, rather than a
        // nudge every three minutes at a worker nobody has heard from.
        for offset in [300, 900, 2_100] {
            let plan = memory.plan(&[busy()], &armed, at(1_000 + offset)).unwrap();
            assert_eq!(
                plan.nudges
                    .iter()
                    .map(|nudge| nudge.text())
                    .collect::<Vec<_>>(),
                vec!["niles: no report from impl — check workers"],
                "at {offset}s"
            );
            assert!(plan.disarms.is_empty());

            // The re-arm only happens because the nudge landed; it is what stops the next tick
            // from firing the same check-in again.
            let retry = memory
                .plan(&[busy()], &armed, at(1_001i64.checked_add(offset).unwrap()))
                .unwrap();
            assert_eq!(only(&plan.nudges).text(), only(&retry.nudges).text());
            assert_ne!(only(&plan.nudges), only(&retry.nudges));
            let nudge = only(&plan.nudges).clone();
            memory.commit(&nudge);
            let next = match nudge {
                Item::Checkin { next, .. } => next,
                Item::Reported { .. } => None,
            }
            .expect("a check-in nudge must carry its re-arm");
            armed = BTreeMap::from([("impl".to_owned(), next)]);
        }
    }

    #[test]
    fn an_actionable_line_past_the_armed_length_disarms_and_an_older_one_does_not() {
        let now = at(1_000);
        let checkins = armed_over(WINDOW, 300, now);
        let mut memory = fresh();

        for extra in [0, 10] {
            let mut stale = armed_over(&format!("{WINDOW}{DONE}"), 300, now);
            stale.get_mut("impl").unwrap().armed_len += extra;
            let plan = memory
                .plan(
                    &[worker("impl", &format!("{WINDOW}{DONE}"))],
                    &stale,
                    at(1300),
                )
                .unwrap();
            assert!(plan.disarms.is_empty(), "{plan:?}");
            memory.commit(plan.nudges.first().unwrap());
        }

        // A report that lands after arming does answer it.
        let plan = memory
            .plan(
                &[worker("impl", &format!("{WINDOW}{DONE}done: answered\n"))],
                &checkins,
                at(1300),
            )
            .unwrap();
        assert_eq!(plan.disarms.len(), 1, "{plan:?}");
    }

    #[test]
    fn a_worker_that_is_gone_is_forgotten() {
        let now = at(1_000);
        let mut memory = fresh();
        let report = worker("impl", &format!("{WINDOW}{DONE}"));

        let gone = memory.plan(&[], &BTreeMap::new(), now).unwrap();
        assert!(gone.nudges.is_empty(), "{gone:?}");

        // A worker id that appears again is a new incarnation — it was closed and respawned — so
        // its log is read from the start rather than against the dead worker's length.
        let plan = memory.plan(&[report], &BTreeMap::new(), at(1001)).unwrap();
        assert_eq!(reports(&plan), vec![reported("impl", "done")]);
    }
}
