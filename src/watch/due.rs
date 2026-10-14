use super::{Sink, checkin::Checkin, decide::WatchMemory, with_planned_checkin};
use crate::worker::WorkerSnapshot;
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

pub(super) fn check_due(
    snapshot: &mut [WorkerSnapshot],
    checkins: &mut BTreeMap<String, Checkin>,
    now: DateTime<Utc>,
    memory: &mut WatchMemory,
    sink: &mut dyn Sink,
) {
    for worker in snapshot {
        let Some(checkin) = checkins.get(&worker.id).copied() else {
            continue;
        };
        if now < checkin.deadline
            || worker
                .last_actionable
                .is_some_and(|wake| checkin.answered_by(wake))
        {
            continue;
        }
        let state = match memory.activity.get(&worker.id) {
            Some((deadline, state)) if *deadline == checkin.deadline => *state,
            _ => match sink.activity(worker, now) {
                Ok(activity) => {
                    if let Some(blocker) = &activity.blocker {
                        match super::report::report(worker, checkin, blocker) {
                            Ok(true) => continue,
                            Ok(false) => {}
                            Err(err) => {
                                sink.note(&format!("failed to report {}: {err:#}", worker.id));
                                checkins.remove(&worker.id);
                                continue;
                            }
                        }
                    }
                    Some(activity.state)
                }
                // An I/O error is not evidence of a terminal turn: no `blocked:` line. It
                // nudges and re-arms like a busy worker, so a lasting one still reaches the lead.
                Err(err) => {
                    sink.note(&format!(
                        "failed to read activity for {}: {err:#}",
                        worker.id
                    ));
                    None
                }
            },
        };
        memory
            .activity
            .insert(worker.id.clone(), (checkin.deadline, state));
        if let Some(held) = checkin.held(now, state) {
            with_planned_checkin(
                &worker.id,
                &worker.worker_dir,
                &checkin,
                sink,
                |dir, sink| {
                    if let Err(err) = held.write(dir) {
                        sink.note(&format!(
                            "failed to hold the check-in for {}: {err:#}",
                            worker.id
                        ));
                    }
                },
            );
            checkins.insert(worker.id.clone(), held);
        }
    }
}

pub(super) fn read_checkins(
    snapshot: &[WorkerSnapshot],
    sink: &mut dyn Sink,
) -> BTreeMap<String, Checkin> {
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
