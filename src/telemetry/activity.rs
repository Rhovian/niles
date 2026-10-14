use super::{SessionLink, SessionState, Usage};
use crate::watch;
use SessionState::{Starting, Unknown, Waiting, Working};
use anyhow::Result;
use camino::Utf8Path;
use chrono::{DateTime, TimeDelta, Utc};

#[derive(Debug)]
pub(crate) struct SessionFailure {
    pub(crate) at: DateTime<Utc>,
    pub(crate) message: String,
}

#[derive(Debug)]
pub(crate) enum Turn {
    InProgress,
    Ended,
    Failed(SessionFailure),
}

pub(crate) struct SessionRecord {
    pub(crate) turn: Option<Turn>,
    pub(crate) usage: Option<Usage>,
}

/// Why a due check-in reports `blocked:` for the worker instead of nudging about it.
#[derive(Debug)]
pub(crate) enum Blocker {
    Refused(SessionFailure),
    NoRecord,
}

pub(crate) struct ResolvedSession {
    pub(crate) state: SessionState,
    pub(crate) blocker: Option<Blocker>,
    pub(crate) usage: Option<Usage>,
}

pub(crate) fn initial_deadline(
    created_at: DateTime<Utc>,
    delay: Option<std::time::Duration>,
) -> DateTime<Utc> {
    let delay = match delay {
        Some(delay) => delay,
        None => watch::DEFAULT_DELAY,
    };
    created_at + TimeDelta::seconds(delay.as_secs() as i64)
}

pub(crate) fn resolve_session(
    link: Option<&SessionLink>,
    workspace: &Utf8Path,
    needle: &Utf8Path,
    created_at: DateTime<Utc>,
    starting_until: DateTime<Utc>,
    now: DateTime<Utc>,
    liveness: impl FnOnce() -> Result<Option<bool>>,
) -> Result<ResolvedSession> {
    let record = link
        .map(|link| super::read_session(link, workspace, needle, created_at))
        .transpose()?
        .flatten();
    let (turn, usage) = match record {
        Some(record) => (record.turn, record.usage),
        None => (None, None),
    };
    let (state, blocker) = match turn {
        Some(Turn::InProgress) => (
            match liveness()? {
                Some(true) => Working,
                Some(false) => Waiting,
                None => Unknown,
            },
            None,
        ),
        Some(Turn::Ended) => (Waiting, None),
        Some(Turn::Failed(failure)) => (Waiting, Some(Blocker::Refused(failure))),
        None if now < starting_until => (Starting, None),
        None => (Unknown, Some(Blocker::NoRecord)),
    };
    Ok(ResolvedSession {
        state,
        blocker,
        usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{at, temp_test_path};
    #[test]
    fn session_lifecycle_requires_turn_evidence_deadline_and_liveness() {
        let dir = temp_test_path("session-lifecycle");
        std::fs::create_dir_all(&dir).unwrap();
        let link = SessionLink::Pi {
            session_dir: dir.clone(),
        };
        let path = dir.join("session.jsonl");
        let resolve = |now, alive| {
            let session =
                resolve_session(Some(&link), &dir, &dir, at(1000), at(1300), at(now), || {
                    Ok(alive)
                })
                .unwrap();
            (session.state, session.blocker.is_some())
        };
        assert_eq!(resolve(1299, Some(true)), (Starting, false));
        assert_eq!(resolve(1300, Some(true)), (Unknown, true));
        assert_eq!(Unknown.displayed(), Waiting);
        std::fs::write(&path, concat!(r#"{"type":"message","timestamp":"1970-01-01T00:16:40Z","message":{"role":"user"}}"#, "\n")).unwrap();
        assert_eq!(resolve(100000, Some(true)), (Working, false));
        assert_eq!(resolve(100000, Some(false)), (Waiting, false));
        assert_eq!(resolve(100000, None), (Unknown, false));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
