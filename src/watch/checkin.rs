//! Check-in arm state: `.niles/worker/<id>/checkin`.
//!
//! Only the lead arms a check-in (`spawn`, `send`); anything else can only disarm it by reporting.
//! The file outlives nothing but the worker's own directory, so it carries everything a later
//! tick needs: when the check-in is due, how long this arm waits, which policy a fire follows,
//! which step of the schedule that is, and — the one that matters — how long the status log was at
//! the moment it was armed. That last number is what stops a worker's stale previous `done:` from
//! satisfying a fresh assignment.
//!
//! The policy travels in the state rather than in the manifest the watcher would have to re-read:
//! the watcher's job is to fire what is armed, and a workspace whose manifest changed mid-flight
//! would otherwise re-time a check-in that was armed under the old one.
//!
//! No lock guards it and none is wanted: the stake is one extra look. A tick that reads the file
//! mid-write treats it as unreadable and tries again a second later.

use std::{fs, io::ErrorKind, time::Duration};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};

use crate::{worker::ActionableWake, workspace_manifest::WorkspaceManifest};

/// The delay `spawn` and `send` arm when nothing else is asked for.
pub(crate) const DEFAULT_DELAY: Duration = Duration::from_secs(5 * 60);

/// How far a doubling re-check backs off.
///
/// A worker still silent after an hour is worth a look, but it is not worth a nudge every three
/// minutes to surface: past this the gap stops growing rather than the nudge stopping.
pub(crate) const BACKOFF_CAP: Duration = Duration::from_secs(60 * 60);

/// How a check-in that has fired picks the delay it re-arms at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Recheck {
    /// Double the delay after each fire, up to [`BACKOFF_CAP`]: 5m, 10m, 20m, 40m, 60m, 60m…
    Backoff,
    /// The same delay after every fire, because the manifest asked for a fixed one.
    Fixed(Duration),
}

impl Recheck {
    /// The delay the next check-in waits, given the one that just fired.
    fn next_delay(self, fired: Duration) -> Duration {
        match self {
            Self::Backoff => {
                // Doubled, bounded by the cap, and never below the delay that just fired: an
                // explicit `--checkin 2h` is past the cap already, and the cap is a ceiling on
                // growth rather than a re-write of the delay the lead asked for.
                fired.saturating_mul(2).min(BACKOFF_CAP).max(fired)
            }
            Self::Fixed(delay) => delay,
        }
    }

    /// How the arm state spells this policy: `backoff`, or the delay as `--checkin` spells it.
    fn spelling(self) -> String {
        match self {
            Self::Backoff => "backoff".to_owned(),
            Self::Fixed(delay) => describe_delay(delay),
        }
    }
}

/// The check-in a dispatch arms with: how long it waits, and what a fire does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cadence {
    /// `None` is no check-in at all: `--checkin off`, or `checkin: off` in the manifest.
    pub(crate) delay: Option<Duration>,
    pub(crate) recheck: Recheck,
}

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
    /// The delay this arm waits, in seconds: the one `--checkin` asked for, or the one the last
    /// fire re-armed at. Carried because a re-arm doubles *it*, not the schedule's first step.
    pub(crate) delay: u64,
    /// Seconds since arming at which this check-in is due: 300, then 900, then 2100…
    pub(crate) step: u64,
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
            step: delay,
            recheck,
            armed_len,
        }
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
            step: self.step + delay,
            recheck: self.recheck,
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
        let mut delay = None;
        let mut step = None;
        let mut recheck = None;
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
                "delay" => {
                    delay = Some(
                        value
                            .parse::<u64>()
                            .with_context(|| format!("invalid check-in delay in {path}"))?,
                    );
                }
                "step" => {
                    step = Some(
                        value
                            .parse::<u64>()
                            .with_context(|| format!("invalid check-in step in {path}"))?,
                    );
                }
                "recheck" => {
                    recheck = Some(
                        parse_recheck(value)
                            .with_context(|| format!("invalid check-in re-check in {path}"))?,
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

        match (deadline, delay, step, recheck, armed_len) {
            (Some(deadline), Some(delay), Some(step), Some(recheck), Some(armed_len)) => {
                Ok(Some(Self {
                    deadline,
                    delay,
                    step,
                    recheck,
                    armed_len,
                }))
            }
            _ => bail!("check-in file {path} is incomplete; `niles quiet` clears it"),
        }
    }

    /// Written beside the target and renamed over it, rather than truncated in place: a crash
    /// mid-write would otherwise leave a file that no longer parses, and an unreadable check-in is
    /// silently skipped by every later tick — the feature would be off for this worker for good.
    pub(crate) fn write(&self, worker_dir: &Utf8Path) -> Result<()> {
        let path = worker_dir.join(CHECKIN_FILE);
        let body = format!(
            "deadline={}\ndelay={}\nstep={}\nrecheck={}\narmed_len={}\n",
            self.deadline.to_rfc3339_opts(SecondsFormat::Secs, true),
            self.delay,
            self.step,
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

/// `now` plus a delay bounded by [`MAX_DELAY`], so the conversion cannot overflow.
fn fire_at(now: DateTime<Utc>, seconds: u64) -> DateTime<Utc> {
    now + TimeDelta::seconds(seconds as i64)
}

/// The cadence a dispatch arms with, from the `--checkin` flag and the workspace manifest.
///
/// Precedence is the flag, then the manifest's `checkin`, then [`DEFAULT_DELAY`]. The re-check
/// policy is the manifest's `recheck`, and backoff when it says nothing. A malformed value in
/// either key is refused here rather than defaulted over: the lead would otherwise be printed a
/// cadence they never asked for.
pub(crate) fn resolve_cadence(
    flag: Option<&str>,
    manifest: Option<&WorkspaceManifest>,
    manifest_path: &Utf8Path,
) -> Result<Cadence> {
    let delay = match flag {
        Some(value) => parse_delay(value)?,
        None => match manifest.and_then(|manifest| manifest.checkin.as_deref()) {
            Some(value) => parse_delay(value).with_context(|| {
                format!("invalid `checkin` in workspace manifest {manifest_path}")
            })?,
            None => Some(DEFAULT_DELAY),
        },
    };
    let recheck = match manifest.and_then(|manifest| manifest.recheck.as_deref()) {
        Some(value) => parse_recheck(value)
            .with_context(|| format!("invalid `recheck` in workspace manifest {manifest_path}"))?,
        None => Recheck::Backoff,
    };

    Ok(Cadence { delay, recheck })
}

/// The `recheck` spelling: the literal `backoff`, or a delay like `10m` to re-arm flat.
fn parse_recheck(value: &str) -> Result<Recheck> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("backoff") {
        return Ok(Recheck::Backoff);
    }

    match parse_delay(value)? {
        Some(delay) => Ok(Recheck::Fixed(delay)),
        None => bail!(
            "re-check policy `{value}` is not `backoff` or a delay; use backoff, 10m, 1h, or a \
             bare number of minutes"
        ),
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

    use camino::Utf8PathBuf;

    use super::*;
    use crate::test_support::temp_test_path;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(seconds, 0).unwrap()
    }

    fn manifest_file() -> Utf8PathBuf {
        Utf8PathBuf::from("/w/.niles/manifest.yaml")
    }

    /// A workspace manifest carrying the two check-in keys under test.
    fn manifest(checkin: Option<&str>, recheck: Option<&str>) -> WorkspaceManifest {
        WorkspaceManifest {
            checkin: checkin.map(str::to_owned),
            recheck: recheck.map(str::to_owned),
            ..WorkspaceManifest::default()
        }
    }

    fn cadence(flag: Option<&str>, manifest: Option<&WorkspaceManifest>) -> Result<Cadence> {
        resolve_cadence(flag, manifest, &manifest_file())
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
        assert_eq!(cadence(None, None).unwrap().delay, Some(DEFAULT_DELAY));
    }

    /// The precedence chain: the flag wins over the manifest, the manifest over the default. The
    /// re-check policy is the manifest's either way, since it says what a *fire* does and not what
    /// this dispatch waits.
    #[test]
    fn the_flag_beats_the_manifest_which_beats_the_default() {
        let configured = manifest(Some("15m"), Some("30m"));

        assert_eq!(
            cadence(None, Some(&configured)).unwrap(),
            Cadence {
                delay: Some(Duration::from_secs(900)),
                recheck: Recheck::Fixed(Duration::from_secs(1800)),
            }
        );
        assert_eq!(
            cadence(Some("90s"), Some(&configured)).unwrap(),
            Cadence {
                delay: Some(Duration::from_secs(90)),
                recheck: Recheck::Fixed(Duration::from_secs(1800)),
            }
        );
        // No flag and no manifest key is the five-minute default; a manifest that says nothing
        // about re-checks gets the backoff.
        assert_eq!(
            cadence(None, Some(&manifest(None, None))).unwrap(),
            Cadence {
                delay: Some(DEFAULT_DELAY),
                recheck: Recheck::Backoff,
            }
        );
        // `--checkin off` is a decision about this dispatch and is not overridden by the manifest.
        assert_eq!(cadence(Some("off"), Some(&configured)).unwrap().delay, None);
    }

    #[test]
    fn a_malformed_manifest_value_is_refused_rather_than_defaulted_over() {
        for (manifest, key) in [
            (manifest(Some("soon"), None), "checkin"),
            (manifest(None, Some("fast")), "recheck"),
            (manifest(None, Some("off")), "recheck"),
            (manifest(None, Some("daily")), "recheck"),
        ] {
            let err = format!("{:#}", cadence(None, Some(&manifest)).unwrap_err());
            assert!(err.contains(key), "{err}");
        }

        for value in ["backoff", "BACKOFF", "10m", "1h", "7"] {
            assert!(
                cadence(None, Some(&manifest(None, Some(value)))).is_ok(),
                "{value}"
            );
        }
    }

    /// A manifest typo names the file it is in, so the lead can go and fix it.
    #[test]
    fn a_manifest_typo_names_the_manifest() {
        let err = format!(
            "{:#}",
            cadence(None, Some(&manifest(Some("soon"), None))).unwrap_err()
        );

        assert!(err.contains(manifest_file().as_str()), "{err}");
        assert!(err.contains("is not a duration"), "{err}");
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
        let armed = Checkin::armed(Duration::from_secs(65), Recheck::Backoff, 0, at(1_000));
        assert_eq!(armed.elapsed_label(), "65s");
        assert_eq!(armed.rearmed(at(1_065)).elapsed_label(), "195s");
    }

    /// The re-check schedule: each fire arms at twice the delay that just fired, so the gap grows
    /// instead of the nudge repeating. The step is what the nudge words, and it stays the silence
    /// since the assignment — 5m, then 15m, then 35m — so a slower cadence does not make the lead
    /// read a smaller number than the worker has been quiet for.
    #[test]
    fn arming_keeps_the_log_length_and_doubles_each_fire_up_to_the_cap() {
        let now = at(1_000);
        let mut armed = Checkin::armed(Duration::from_secs(300), Recheck::Backoff, 42, now);

        assert_eq!(armed.deadline, at(1_300));
        assert_eq!(armed.delay, 300);
        assert_eq!(armed.elapsed_label(), "5m");
        assert_eq!(armed.armed_len, 42);

        // deadline, delay, silence so far: 5m -> 10m -> 20m -> 40m -> 60m, then 60m forever.
        for (deadline, delay, step) in [
            (1_900, 600, 900),
            (3_100, 1_200, 2_100),
            (5_500, 2_400, 4_500),
            (9_100, 3_600, 8_100),
            (12_700, 3_600, 11_700),
        ] {
            let fired = armed;
            armed = fired.rearmed(fired.deadline);

            assert_eq!(armed.deadline, at(deadline), "{delay}s arm");
            assert_eq!(armed.delay, delay);
            assert_eq!(armed.step, step);
            assert_eq!(
                armed.elapsed_label(),
                describe_delay(Duration::from_secs(step))
            );
        }

        assert!(armed.answered_by(ActionableWake {
            end: 43,
            kind: crate::wake::WakeKind::Done
        }));
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

    /// `recheck: 10m` in the manifest: the same delay after every fire, so only the silence the
    /// nudge reports keeps growing.
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
        assert_eq!(fired.elapsed_label(), "15m");
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

    /// A policy the state file cannot express is unreadable arm state rather than a policy to
    /// guess at: the watcher fires what is armed and nothing else.
    #[test]
    fn an_unreadable_recheck_policy_is_an_error_rather_than_a_default() {
        let dir = temp_test_path("bad-recheck");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(CHECKIN_FILE),
            "deadline=1970-01-01T00:16:40Z\ndelay=300\nstep=300\nrecheck=soon\narmed_len=0\n",
        )
        .unwrap();

        let err = Checkin::read(&dir).unwrap_err();

        assert!(err.to_string().contains("re-check"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
