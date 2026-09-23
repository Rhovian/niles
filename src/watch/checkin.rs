//! Check-in arm state: `.niles/worker/<id>/checkin`.
//!
//! Only the lead arms a check-in (`spawn`, `send`); anything else can only disarm it by reporting.
//! The file outlives nothing but the worker's own directory, so it carries everything a later
//! tick needs: when the check-in is due, which step of the backoff that is, and — the one that
//! matters — how long the status log was at the moment it was armed. That last number is what
//! stops a worker's stale previous `done:` from satisfying a fresh assignment.
//!
//! No lock guards it and none is wanted: the stake is one extra look. A tick that reads the file
//! mid-write treats it as unreadable and tries again a second later.

use std::{fs, io::ErrorKind, time::Duration};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};

use crate::worker::ActionableWake;

/// The delay `spawn` and `send` arm when nothing else is asked for.
pub(crate) const DEFAULT_DELAY: Duration = Duration::from_secs(5 * 60);

/// How much later a check-in re-arms once it has fired.
///
/// Check-ins continue until someone looks, which is the point: a worker that reported and then
/// went quiet is still a worker the lead has stopped hearing from.
pub(crate) const RECHECK_DELAY: Duration = Duration::from_secs(3 * 60);

/// Longest accepted `--checkin`. A check-in further out than a day is always a typo.
const MAX_DELAY: Duration = Duration::from_secs(24 * 60 * 60);

const CHECKIN_FILE: &str = "checkin";

/// Where a check-in is written before it is renamed into place. Never read: a leftover one is a
/// write that never landed, and the next write overwrites it.
const STAGING_FILE: &str = "checkin.tmp";

/// One worker's check-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Checkin {
    pub(crate) deadline: DateTime<Utc>,
    /// Seconds since arming at which this check-in is due: 300, then 480, then 660…
    pub(crate) step: u64,
    /// Status-log byte length at the moment of arming. An actionable line past it is the worker
    /// answering *this* assignment; anything at or before it is history.
    pub(crate) armed_len: u64,
}

impl Checkin {
    pub(crate) fn armed(delay: Duration, armed_len: u64, now: DateTime<Utc>) -> Self {
        let step = delay.as_secs();
        Self {
            deadline: fire_at(now, step),
            step,
            armed_len,
        }
    }

    /// The check-in that follows this one, once it has fired.
    pub(crate) fn rearmed(&self, now: DateTime<Utc>) -> Self {
        Self {
            deadline: fire_at(now, RECHECK_DELAY.as_secs()),
            step: self.step + RECHECK_DELAY.as_secs(),
            armed_len: self.armed_len,
        }
    }

    /// How long since arming this check-in is due at, as the nudge words it.
    ///
    /// Rendered by the same speller `--checkin` echoes back, so a check-in armed at `65s` is
    /// reported as `65s` rather than rounded up to a minute it has not reached. The nudge's whole
    /// job is to say how long it has been.
    pub(crate) fn elapsed_label(&self) -> String {
        describe_delay(Duration::from_secs(self.step))
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

        let mut deadline = None;
        let mut step = None;
        let mut armed_len = None;
        for line in body.lines() {
            let Some((key, value)) = line.trim().split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key {
                "deadline" => {
                    deadline = Some(
                        DateTime::parse_from_rfc3339(value)
                            .with_context(|| format!("invalid check-in deadline in {path}"))?
                            .with_timezone(&Utc),
                    );
                }
                "step" => {
                    step = Some(
                        value
                            .parse::<u64>()
                            .with_context(|| format!("invalid check-in step in {path}"))?,
                    );
                }
                "armed_len" => {
                    armed_len = Some(
                        value
                            .parse::<u64>()
                            .with_context(|| format!("invalid check-in log length in {path}"))?,
                    );
                }
                _ => {}
            }
        }

        match (deadline, step, armed_len) {
            (Some(deadline), Some(step), Some(armed_len)) => Ok(Some(Self {
                deadline,
                step,
                armed_len,
            })),
            _ => bail!("check-in file {path} is incomplete; `niles quiet` clears it"),
        }
    }

    /// Written beside the target and renamed over it, rather than truncated in place: a crash
    /// mid-write would otherwise leave a file that no longer parses, and an unreadable check-in is
    /// silently skipped by every later tick — the feature would be off for this worker for good.
    pub(crate) fn write(&self, worker_dir: &Utf8Path) -> Result<()> {
        let path = worker_dir.join(CHECKIN_FILE);
        let body = format!(
            "deadline={}\nstep={}\narmed_len={}\n",
            self.deadline.to_rfc3339_opts(SecondsFormat::Secs, true),
            self.step,
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

/// `now` plus a delay bounded by [`MAX_DELAY`], so the conversion cannot overflow.
fn fire_at(now: DateTime<Utc>, seconds: u64) -> DateTime<Utc> {
    now + TimeDelta::seconds(seconds as i64)
}

/// Resolves the `--checkin` value: `90s`, `5m`, `1h`, a bare integer as minutes, or `0`/`off` to
/// arm nothing at all. No flag is the five-minute default.
pub(crate) fn resolve_delay(flag: Option<&str>) -> Result<Option<Duration>> {
    match flag {
        Some(value) => parse_delay(value),
        None => Ok(Some(DEFAULT_DELAY)),
    }
}

/// The `--checkin` spelling of a delay, for `spawn` to print back to the lead.
pub(crate) fn describe_delay(delay: Duration) -> String {
    let seconds = delay.as_secs();
    if seconds.is_multiple_of(60 * 60) {
        format!("{}h", seconds / (60 * 60))
    } else if seconds.is_multiple_of(60) {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

fn parse_delay(value: &str) -> Result<Option<Duration>> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("off") {
        return Ok(None);
    }

    let (digits, unit) = match value.chars().last() {
        Some(unit) if unit.is_ascii_alphabetic() => (&value[..value.len() - 1], Some(unit)),
        _ => (value, None),
    };
    let Ok(amount) = digits.trim().parse::<u64>() else {
        bail!(
            "check-in delay `{value}` is not a duration; use 90s, 5m, 1h, a bare number of \
             minutes, or 0/off"
        );
    };
    let multiplier = match unit {
        None => 60,
        Some('s') => 1,
        Some('m') => 60,
        Some('h') => 60 * 60,
        Some(other) => bail!("check-in delay `{value}` uses unknown unit `{other}`; use s, m or h"),
    };
    let Some(seconds) = amount.checked_mul(multiplier) else {
        bail!("check-in delay `{value}` is too long");
    };

    let delay = Duration::from_secs(seconds);
    if delay.is_zero() {
        return Ok(None);
    }
    if delay > MAX_DELAY {
        bail!("check-in delay `{value}` is longer than 24h");
    }
    Ok(Some(delay))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;
    use crate::test_support::temp_test_path;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(seconds, 0).unwrap()
    }

    /// The `--checkin` spellings, both directions: what the lead can write, and what `spawn` prints
    /// back for the delay it armed. A bare number is minutes, and describes back as minutes —
    /// `7m` — since that is the delay `spawn` actually armed.
    #[test]
    fn the_checkin_spellings_parse_and_the_armed_delay_prints_back() {
        for (written, delay) in [
            ("90s", Duration::from_secs(90)),
            ("5m", Duration::from_secs(300)),
            ("1h", Duration::from_secs(3600)),
            ("7", Duration::from_secs(420)),
        ] {
            assert_eq!(
                parse_delay(written).unwrap(),
                Some(delay),
                "--checkin {written}"
            );
        }

        for (delay, printed) in [
            (Duration::from_secs(90), "90s"),
            (Duration::from_secs(300), "5m"),
            (Duration::from_secs(3600), "1h"),
        ] {
            assert_eq!(describe_delay(delay), printed, "{}s armed", delay.as_secs());
        }
    }

    #[test]
    fn zero_and_off_arm_nothing() {
        assert_eq!(parse_delay("0").unwrap(), None);
        assert_eq!(parse_delay("0s").unwrap(), None);
        assert_eq!(parse_delay("off").unwrap(), None);
        assert_eq!(parse_delay("OFF").unwrap(), None);
        assert_eq!(resolve_delay(None).unwrap(), Some(DEFAULT_DELAY));
    }

    #[test]
    fn a_malformed_delay_is_rejected_rather_than_guessed_at() {
        for value in ["", "5x", "m", "-5", "1.5m", "99999h"] {
            assert!(parse_delay(value).is_err(), "{value} should not parse");
        }
    }

    #[test]
    fn a_delay_that_is_not_whole_minutes_is_reported_as_it_was_asked_for() {
        // `--checkin 65s` used to nudge "no report ... in 2m" — a minute the check-in had not
        // reached, in the one sentence whose job is to say how long it has been.
        let armed = Checkin::armed(Duration::from_secs(65), 0, at(1_000));
        assert_eq!(armed.elapsed_label(), "65s");
        assert_eq!(armed.rearmed(at(1_065)).elapsed_label(), "245s");
    }

    #[test]
    fn arming_keeps_the_log_length_and_steps_three_minutes_each_fire() {
        let now = at(1_000);
        let armed = Checkin::armed(Duration::from_secs(300), 42, now);

        assert_eq!(armed.deadline, at(1_300));
        assert_eq!(armed.elapsed_label(), "5m");
        assert_eq!(armed.armed_len, 42);

        let refired = armed.rearmed(at(1_300));
        assert_eq!(refired.deadline, at(1_480));
        assert_eq!(refired.elapsed_label(), "8m");
        assert!(refired.answered_by(ActionableWake {
            end: 43,
            kind: crate::wake::WakeKind::Done
        }));
    }

    #[test]
    fn only_a_line_past_the_armed_length_answers_a_check_in() {
        let checkin = Checkin::armed(Duration::from_secs(300), 10, at(1_000));
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
        let armed = Checkin::armed(Duration::from_secs(90), 7, at(1_000));

        assert_eq!(Checkin::read(&dir).unwrap(), None);
        armed.write(&dir).unwrap();
        assert_eq!(Checkin::read(&dir).unwrap(), Some(armed));

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
        Checkin::armed(Duration::from_secs(300), 3, at(1_000))
            .write(&dir)
            .unwrap();
        let replacement = Checkin::armed(Duration::from_secs(90), 7, at(2_000));
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
    fn an_incomplete_checkin_file_is_an_error_rather_than_a_default() {
        let dir = temp_test_path("incomplete");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(CHECKIN_FILE),
            "deadline=1970-01-01T00:16:40Z\nstep=300\n",
        )
        .unwrap();

        let err = Checkin::read(&dir).unwrap_err();

        assert!(err.to_string().contains("incomplete"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
