use std::time::Duration;

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::workspace_manifest::WorkspaceManifest;

/// The delay `spawn` and `send` arm when nothing else is asked for.
pub(crate) const DEFAULT_DELAY: Duration = Duration::from_secs(5 * 60);

/// A worker still silent after an hour is worth a look, but not increasingly sparse looks.
const BACKOFF_CAP: Duration = Duration::from_secs(60 * 60);

/// Longest accepted `--checkin`. A check-in further out than a day is always a typo.
const MAX_DELAY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Recheck {
    Backoff,
    Fixed(Duration),
}

impl Recheck {
    pub(super) fn next_delay(self, fired: Duration) -> Duration {
        match self {
            Self::Backoff => fired.saturating_mul(2).min(BACKOFF_CAP).max(fired),
            Self::Fixed(delay) => delay,
        }
    }

    pub(super) fn spelling(self) -> String {
        match self {
            Self::Backoff => "backoff".to_owned(),
            Self::Fixed(delay) => describe_delay(delay),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cadence {
    pub(crate) delay: Option<Duration>,
    pub(crate) recheck: Recheck,
}

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

pub(super) fn parse_recheck(value: &str) -> Result<Recheck> {
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
    use camino::Utf8PathBuf;

    use super::*;

    fn manifest_file() -> Utf8PathBuf {
        Utf8PathBuf::from("/w/.niles/manifest.yaml")
    }

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
        assert_eq!(
            cadence(None, Some(&manifest(None, None))).unwrap(),
            Cadence {
                delay: Some(DEFAULT_DELAY),
                recheck: Recheck::Backoff,
            }
        );
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
            assert!(err.contains(manifest_file().as_str()), "{err}");
        }

        for value in ["backoff", "BACKOFF", "10m", "1h", "7"] {
            assert!(
                cadence(None, Some(&manifest(None, Some(value)))).is_ok(),
                "{value}"
            );
        }
    }

    #[test]
    fn a_malformed_delay_is_rejected_rather_than_guessed_at() {
        for value in ["", "5x", "m", "-5", "1.5m", "99999h"] {
            assert!(parse_delay(value).is_err(), "{value} should not parse");
        }
    }
}
