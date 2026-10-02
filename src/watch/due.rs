use super::{Sink, checkin::Checkin, decide::WatchMemory, with_planned_checkin};
use crate::{
    telemetry::SessionState,
    worker::{self, WorkerSnapshot},
};
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

pub(super) fn check_due(
    snapshot: &[WorkerSnapshot],
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
            _ => read_state(worker, sink),
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

fn read_state(worker: &WorkerSnapshot, sink: &mut dyn Sink) -> Option<SessionState> {
    match worker::worker_usage(worker) {
        Ok(usage) => usage.and_then(|usage| usage.state),
        Err(err) => {
            sink.note(&format!(
                "failed to read telemetry for {}: {err:#}",
                worker.id
            ));
            None
        }
    }
}
