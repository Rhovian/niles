//! Check-in arm state: `.niles/worker/<id>/checkin`.
//!
//! `armed_len` keeps an old report from satisfying a fresh assignment. No lock guards this file:
//! a tick that catches a write in progress retries on the next pass.

use std::{collections::BTreeMap, fs, io::ErrorKind, time::Duration};

use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};

use crate::{telemetry::SessionState, worker::ActionableWake};

use super::cadence::{Recheck, parse_recheck};

const CHECKIN_FILE: &str = "checkin";

/// Where a check-in is written before it is renamed into place. Never read: a leftover one is a
/// write that never landed, and the next write overwrites it.
const STAGING_FILE: &str = "checkin.tmp";

/// One worker's check-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Checkin {
    pub(crate) deadline: DateTime<Utc>,
    /// The delay this arm waits, in seconds: the one `--checkin` asked for, or the one the last
    /// fire re-armed at. Carried because a re-arm doubles *it*, not the schedule's first step.
    pub(crate) delay: u64,
    /// The policy this arm follows, so the watcher re-arms from the state alone.
    pub(crate) recheck: Recheck,
    /// Status-log byte length at the moment of arming. An actionable line past it is the worker
    /// answering *this* assignment; anything at or before it is history.
    pub(crate) armed_len: u64,
}

impl Checkin {
    pub(crate) fn armed(
        delay: Duration,
        recheck: Recheck,
        armed_len: u64,
        now: DateTime<Utc>,
    ) -> Self {
        let delay = delay.as_secs();
        Self {
            deadline: fire_at(now, delay),
            delay,
            recheck,
            armed_len,
        }
    }

    pub(crate) fn held(&self, now: DateTime<Utc>, state: Option<SessionState>) -> Option<Self> {
        (state == Some(SessionState::Working)).then_some(Self {
            deadline: fire_at(now, self.delay),
            ..*self
        })
    }

    /// The check-in that follows this one, once it has fired.
    pub(crate) fn rearmed(&self, now: DateTime<Utc>) -> Self {
        let delay = self
            .recheck
            .next_delay(Duration::from_secs(self.delay))
            .as_secs();
        Self {
            deadline: fire_at(now, delay),
            delay,
            recheck: self.recheck,
            armed_len: self.armed_len,
        }
    }

    /// Whether `wake` answers the assignment this check-in was armed for.
    pub(crate) fn answered_by(&self, wake: ActionableWake) -> bool {
        wake.end > self.armed_len
    }

    pub(crate) fn read(worker_dir: &Utf8Path) -> Result<Option<Self>> {
        let path = worker_dir.join(CHECKIN_FILE);
        let body = match fs::read_to_string(&path) {
            Ok(body) => body,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).with_context(|| format!("failed to read {path}")),
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
            delay: field(&fields, "delay", "delay", &path, str::parse::<u64>)?,
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
            self.delay,
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

/// `now` plus a parsed delay, which is bounded so the conversion cannot overflow.
fn fire_at(now: DateTime<Utc>, seconds: u64) -> DateTime<Utc> {
    now + TimeDelta::seconds(seconds as i64)
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
    use crate::test_support::{at, temp_test_path};

    #[test]
    fn working_holds_and_waiting_or_unknown_fires() {
        let checkin = Checkin::armed(Duration::from_secs(300), Recheck::Backoff, 0, at(1_000));
        let held = checkin
            .held(at(1_300), Some(SessionState::Working))
            .unwrap();
        assert_eq!(held.deadline, at(1_600));
        assert!(
            checkin
                .held(at(1_300), Some(SessionState::Waiting))
                .is_none()
        );
        assert!(checkin.held(at(1_300), None).is_none());
    }

    /// The re-check schedule: each fire arms at twice the delay that just fired, so the gap grows
    /// instead of the nudge repeating.
    #[test]
    fn arming_keeps_the_log_length_and_doubles_each_fire_up_to_the_cap() {
        let now = at(1_000);
        let mut armed = Checkin::armed(Duration::from_secs(300), Recheck::Backoff, 42, now);

        assert_eq!(armed.deadline, at(1_300));
        assert_eq!(armed.delay, 300);
        assert_eq!(armed.armed_len, 42);

        // deadline, delay, silence so far: 5m -> 10m -> 20m -> 40m -> 60m, then 60m forever.
        for (deadline, delay) in [
            (1_900, 600),
            (3_100, 1_200),
            (5_500, 2_400),
            (9_100, 3_600),
            (12_700, 3_600),
        ] {
            let fired = armed;
            armed = fired.rearmed(fired.deadline);

            assert_eq!(armed.deadline, at(deadline), "{delay}s arm");
            assert_eq!(armed.delay, delay);
        }
    }

    /// A first delay already past the cap — an explicit `--checkin 2h` — is not shortened by it.
    /// The cap bounds how far the gap grows; it does not re-write the delay the lead asked for.
    #[test]
    fn the_cap_never_shortens_a_delay_longer_than_it() {
        let armed = Checkin::armed(
            Duration::from_secs(2 * 60 * 60),
            Recheck::Backoff,
            0,
            at(1_000),
        );

        assert_eq!(armed.rearmed(at(8_200)).delay, 2 * 60 * 60);
    }

    /// `recheck: 10m` in the manifest: the same delay after every fire.
    #[test]
    fn a_fixed_recheck_arms_the_same_delay_every_fire() {
        let armed = Checkin::armed(
            Duration::from_secs(300),
            Recheck::Fixed(Duration::from_secs(600)),
            0,
            at(1_000),
        );

        let fired = armed.rearmed(at(1_300));
        assert_eq!(fired.deadline, at(1_900));
        assert_eq!(fired.delay, 600);
        assert_eq!(fired.rearmed(at(1_900)).deadline, at(2_500));
    }

    #[test]
    fn only_a_line_past_the_armed_length_answers_a_check_in() {
        let checkin = Checkin::armed(Duration::from_secs(300), Recheck::Backoff, 10, at(1_000));
        let wake = |end| ActionableWake {
            end,
            kind: crate::wake::WakeKind::Done,
        };

        assert!(!checkin.answered_by(wake(10)));
        assert!(!checkin.answered_by(wake(0)));
        assert!(checkin.answered_by(wake(11)));
    }

    #[test]
    fn arm_state_round_trips_through_the_worker_directory() {
        let dir = temp_test_path("round-trip");
        fs::create_dir_all(&dir).unwrap();
        let armed = Checkin::armed(
            Duration::from_secs(90),
            Recheck::Fixed(Duration::from_secs(600)),
            7,
            at(1_000),
        );

        assert_eq!(Checkin::read(&dir).unwrap(), None);
        armed.write(&dir).unwrap();
        assert_eq!(Checkin::read(&dir).unwrap(), Some(armed));

        // The policy and the delay a re-arm doubles are on disk, so the watcher re-arms from the
        // state alone: it never needs the manifest the check-in was armed under.
        let body = fs::read_to_string(dir.join(CHECKIN_FILE)).unwrap();
        assert!(body.contains("delay=90"), "{body}");
        assert!(body.contains("recheck=10m"), "{body}");
        let next = armed.rearmed(at(1_090));
        next.write(&dir).unwrap();
        assert_eq!(Checkin::read(&dir).unwrap(), Some(next));
        assert_eq!(next.delay, 600);

        assert!(Checkin::disarm(&dir).unwrap());
        assert_eq!(Checkin::read(&dir).unwrap(), None);
        // Nothing to delete is a disarm that already happened, not a failure.
        assert!(!Checkin::disarm(&dir).unwrap());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_write_replaces_the_file_whole_and_leaves_no_staging_file() {
        let dir = temp_test_path("staging");
        fs::create_dir_all(&dir).unwrap();
        Checkin::armed(Duration::from_secs(300), Recheck::Backoff, 3, at(1_000))
            .write(&dir)
            .unwrap();
        let replacement = Checkin::armed(Duration::from_secs(90), Recheck::Backoff, 7, at(2_000));
        let path = dir.join(CHECKIN_FILE);
        let before = fs::metadata(&path).unwrap().ino();

        replacement.write(&dir).unwrap();

        // Written beside the file and renamed over it, so a reader never sees half of either check-in.
        // The inode moving is what says it was replaced rather than truncated in place.
        assert_ne!(
            fs::metadata(&path).unwrap().ino(),
            before,
            "the check-in must be replaced, not truncated in place"
        );
        assert_eq!(Checkin::read(&dir).unwrap(), Some(replacement));
        assert!(!dir.join(STAGING_FILE).exists());

        // A staging file a crashed write left behind is not arm state, and the real file still reads.
        fs::write(dir.join(STAGING_FILE), "deadline=half-a-write").unwrap();
        assert_eq!(Checkin::read(&dir).unwrap(), Some(replacement));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn malformed_checkin_files_are_errors_rather_than_defaults() {
        for (label, body, expected) in [
            (
                "incomplete",
                "deadline=1970-01-01T00:16:40Z\n",
                "incomplete",
            ),
            (
                "bad-recheck",
                "deadline=1970-01-01T00:16:40Z\ndelay=300\nrecheck=soon\narmed_len=0\n",
                "re-check",
            ),
        ] {
            let dir = temp_test_path(label);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(CHECKIN_FILE), body).unwrap();

            let err = Checkin::read(&dir).unwrap_err();

            assert!(err.to_string().contains(expected), "{err}");
            fs::remove_dir_all(&dir).unwrap();
        }
    }
}
