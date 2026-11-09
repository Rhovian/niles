//! Check-in arm state: `worker/<id>/checkin` in global project state.
//!
//! `armed_len` keeps an old report from satisfying a fresh assignment. Every write and removal
//! happens under [`Checkin::lock`]; a plain read takes no lock, because writes rename a whole file
//! into place.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::ErrorKind,
};

use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};

use crate::{telemetry::SessionState, wait::cursor::open_lock_file, worker::ActionableWake};

use super::cadence::{Delay, Recheck, parse_delay, parse_recheck};

const CHECKIN_FILE: &str = "checkin";
const LOCK_FILE: &str = "checkin.lock";

/// Where a check-in is written before it is renamed into place. Never read: a leftover one is a
/// write that never landed, and the next write overwrites it.
const STAGING_FILE: &str = "checkin.tmp";

/// One worker's check-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Checkin {
    pub(crate) deadline: DateTime<Utc>,
    /// The delay this arm waits, in seconds: the one `--checkin` asked for, or the one the last
    /// fire re-armed at. Carried because a re-arm doubles *it*, not the schedule's first step.
    pub(crate) delay: Delay,
    /// The policy this arm follows, so the watcher re-arms from the state alone.
    pub(crate) recheck: Recheck,
    /// Status-log byte length at the moment of arming. An actionable line past it is the worker
    /// answering *this* assignment; anything at or before it is history.
    pub(crate) armed_len: u64,
}

impl Checkin {
    pub(crate) fn armed(
        delay: Delay,
        recheck: Recheck,
        armed_len: u64,
        now: DateTime<Utc>,
    ) -> Result<Self> {
        Ok(Self {
            deadline: fire_at(now, delay)?,
            delay,
            recheck,
            armed_len,
        })
    }

    pub(crate) fn held(
        &self,
        now: DateTime<Utc>,
        state: Option<SessionState>,
    ) -> Result<Option<Self>> {
        (state == Some(SessionState::Working))
            .then(|| Self::armed(self.delay, self.recheck, self.armed_len, now))
            .transpose()
    }

    /// The check-in that follows this one, once it has fired.
    pub(crate) fn rearmed(&self, now: DateTime<Utc>) -> Result<Self> {
        Self::armed(
            self.recheck.next_delay(self.delay),
            self.recheck,
            self.armed_len,
            now,
        )
    }

    /// Whether `wake` answers the assignment this check-in was armed for.
    pub(crate) fn answered_by(&self, wake: ActionableWake) -> bool {
        wake.end > self.armed_len
    }

    /// Holds this worker's check-in lock until the returned file is dropped. It makes the watcher's
    /// re-read and the write or removal that follows one step, so a check-in `send` arms cannot
    /// land between them and be removed or overwritten.
    pub(crate) fn lock(worker_dir: &Utf8Path) -> Result<File> {
        let path = worker_dir.join(LOCK_FILE);
        let file = open_lock_file(&path)?;
        file.lock()
            .with_context(|| format!("failed to lock {path}"))?;
        Ok(file)
    }

    pub(crate) fn read(worker_dir: &Utf8Path) -> Result<Option<Self>> {
        let path = worker_dir.join(CHECKIN_FILE);
        let Some(body) = crate::util::read_optional_string(&path)? else {
            return Ok(None);
        };

        let fields = body
            .lines()
            .filter_map(|line| line.trim().split_once('='))
            .map(|(key, value)| (key, value.trim()))
            .collect::<BTreeMap<_, _>>();
        Ok(Some(Self {
            deadline: field(&fields, "deadline", "deadline", &path, |value| {
                DateTime::parse_from_rfc3339(value).map(|time| time.with_timezone(&Utc))
            })?,
            delay: field(&fields, "delay", "delay", &path, |value| -> Result<Delay> {
                parse_delay(&format!("{}s", value.parse::<u64>()?))?
                    .context("check-in delay must be positive")
            })?,
            recheck: field(&fields, "recheck", "re-check", &path, parse_recheck)?,
            armed_len: field(&fields, "armed_len", "log length", &path, str::parse::<u64>)?,
        }))
    }

    /// Written beside the target and renamed over it, rather than truncated in place: a crash
    /// mid-write would otherwise leave a file that no longer parses, and an unreadable check-in is
    /// silently skipped by every later tick — the feature would be off for this worker for good.
    pub(crate) fn write(&self, worker_dir: &Utf8Path) -> Result<()> {
        let path = worker_dir.join(CHECKIN_FILE);
        let body = format!(
            "deadline={}\ndelay={}\nrecheck={}\narmed_len={}\n",
            self.deadline.to_rfc3339_opts(SecondsFormat::Secs, true),
            self.delay.duration().as_secs(),
            self.recheck.spelling(),
            self.armed_len
        );
        let staging = worker_dir.join(STAGING_FILE);
        fs::write(&staging, body).with_context(|| format!("failed to write {staging}"))?;
        fs::rename(&staging, &path).with_context(|| format!("failed to replace {path}"))
    }

    /// Deletes the arm state. `Ok(false)` when there was none — a worker that disarms twice (it
    /// reported, and the lead also ran `quiet`) is not a failure.
    pub(crate) fn disarm(worker_dir: &Utf8Path) -> Result<bool> {
        let path = worker_dir.join(CHECKIN_FILE);
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err).with_context(|| format!("failed to remove {path}")),
        }
    }
}

fn fire_at(now: DateTime<Utc>, delay: Delay) -> Result<DateTime<Utc>> {
    now.checked_add_signed(TimeDelta::from_std(delay.duration())?)
        .context("check-in deadline is out of range")
}

fn field<T, E>(
    fields: &BTreeMap<&str, &str>,
    key: &str,
    what: &str,
    path: &Utf8Path,
    parse: impl FnOnce(&str) -> std::result::Result<T, E>,
) -> Result<T>
where
    E: Into<anyhow::Error>,
{
    let value = fields
        .get(key)
        .with_context(|| format!("check-in file {path} is incomplete; `niles quiet` clears it"))?;
    parse(value)
        .map_err(Into::into)
        .with_context(|| format!("invalid check-in {what} in {path}"))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;
    use crate::test_support::{assert_err_contains, at, temp_dir};

    #[test]
    fn working_holds_and_waiting_or_unknown_fires() {
        let checkin = Checkin::armed(Delay::seconds(300), Recheck::Backoff, 0, at(1_000)).unwrap();
        for (state, deadline) in [
            (Some(SessionState::Working), Some(at(1_600))),
            (Some(SessionState::Waiting), None),
            (None, None),
        ] {
            assert_eq!(
                checkin
                    .held(at(1_300), state)
                    .unwrap()
                    .map(|held| held.deadline),
                deadline
            );
        }
        assert_err_contains(
            checkin.held(DateTime::<Utc>::MAX_UTC, Some(SessionState::Working)),
            "deadline",
        );
    }

    #[test]
    fn rearming_keeps_log_length_and_follows_the_schedule() {
        for (first_delay, recheck, schedule) in [
            (
                300,
                Recheck::Backoff,
                &[
                    (1_300, 300),
                    (1_900, 600),
                    (3_100, 1_200),
                    (5_500, 2_400),
                    (9_100, 3_600),
                    (12_700, 3_600),
                ][..],
            ),
            (
                7_200,
                Recheck::Backoff,
                &[(8_200, 7_200), (15_400, 7_200)][..],
            ),
            (
                300,
                Recheck::Fixed(Delay::seconds(600)),
                &[(1_300, 300), (1_900, 600), (2_500, 600)][..],
            ),
        ] {
            let mut armed =
                Checkin::armed(Delay::seconds(first_delay), recheck, 42, at(1_000)).unwrap();
            for &(deadline, delay) in schedule {
                assert_eq!(armed.deadline, at(deadline));
                assert_eq!(armed.delay.duration().as_secs(), delay);
                assert_eq!(armed.armed_len, 42);
                armed = armed.rearmed(armed.deadline).unwrap();
            }
        }
    }

    #[test]
    fn arm_state_round_trips_and_replaces_whole_files() {
        let dir = temp_dir("round-trip");
        let armed = Checkin::armed(
            Delay::seconds(90),
            Recheck::Fixed(Delay::seconds(600)),
            7,
            at(1_000),
        )
        .unwrap();

        assert_eq!(Checkin::read(&dir).unwrap(), None);
        armed.write(&dir).unwrap();
        assert_eq!(Checkin::read(&dir).unwrap(), Some(armed));

        let body = fs::read_to_string(dir.join(CHECKIN_FILE)).unwrap();
        assert!(body.contains("delay=90"), "{body}");
        assert!(body.contains("recheck=10m"), "{body}");
        let next = armed.rearmed(at(1_090)).unwrap();
        let before = fs::metadata(dir.join(CHECKIN_FILE)).unwrap().ino();
        next.write(&dir).unwrap();
        assert_ne!(fs::metadata(dir.join(CHECKIN_FILE)).unwrap().ino(), before);
        assert!(!dir.join(STAGING_FILE).exists());
        fs::write(dir.join(STAGING_FILE), "deadline=half-a-write").unwrap();
        assert_eq!(Checkin::read(&dir).unwrap(), Some(next));
        assert_eq!(next.delay.duration().as_secs(), 600);

        assert!(Checkin::disarm(&dir).unwrap());
        assert_eq!(Checkin::read(&dir).unwrap(), None);
        // Nothing to delete is a disarm that already happened, not a failure.
        assert!(!Checkin::disarm(&dir).unwrap());
    }

    #[test]
    fn malformed_checkin_files_are_errors_rather_than_defaults() {
        let valid = "deadline=1970-01-01T00:16:40Z\ndelay=300\nrecheck=backoff\narmed_len=0\n";
        for (from, to, expected) in [
            ("delay=300", "delay=0", "delay"),
            ("delay=300", "delay=9223372036854775807", "delay"),
            ("recheck=backoff", "recheck=soon", "re-check"),
            (valid, "deadline=1970-01-01T00:16:40Z\n", "incomplete"),
        ] {
            let dir = temp_dir("malformed-checkin");
            fs::write(dir.join(CHECKIN_FILE), valid.replace(from, to)).unwrap();
            assert_err_contains(Checkin::read(&dir), expected);
        }
    }
}
