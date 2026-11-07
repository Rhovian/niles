use std::time::Duration;

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::{duration::parse_duration, workspace_manifest::WorkspaceManifest};

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

    pub(crate) fn spelling(self) -> String {
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
        Some(value) => parse_delay(value).context("invalid `--checkin`")?,
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

pub(crate) fn parse_recheck(value: &str) -> Result<Recheck> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("backoff") {
        return Ok(Recheck::Backoff);
    }

    match parse_delay(value)? {
        Some(delay) => Ok(Recheck::Fixed(delay)),
        None => bail!(
            "re-check policy `{value}` is not `backoff` or a delay; use backoff, 90s, 10m or 1h"
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

pub(crate) fn parse_delay(value: &str) -> Result<Option<Duration>> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("off") {
        return Ok(None);
    }

    let delay = parse_duration(value)?;
    if delay.is_zero() {
        return Ok(None);
    }
    if delay < Duration::from_secs(1) {
        bail!("check-in delay `{value}` is shorter than 1s");
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
            ..crate::test_support::manifest()
        }
    }

    fn cadence(flag: Option<&str>, manifest: Option<&WorkspaceManifest>) -> Result<Cadence> {
        resolve_cadence(flag, manifest, &manifest_file())
    }

    #[test]
    fn checkin_delays_parse_and_describe() {
        for (input, expected) in [
            ("0", Ok(None)),
            ("0s", Ok(None)),
            ("off", Ok(None)),
            ("OFF", Ok(None)),
            ("1s", Ok(Some(1))),
            ("24h", Ok(Some(86400))),
            ("5x", Err("unknown unit")),
            ("m", Err("not a duration")),
            ("-5", Err("not a duration")),
            ("999ms", Err("shorter than 1s")),
            ("86401s", Err("longer than 24h")),
        ] {
            match expected {
                Ok(seconds) => assert_eq!(
                    parse_delay(input).unwrap(),
                    seconds.map(Duration::from_secs),
                    "{input}"
                ),
                Err(needle) => crate::test_support::assert_err_contains(parse_delay(input), needle),
            }
        }
        for (seconds, printed) in [(90, "90s"), (300, "5m"), (3600, "1h")] {
            assert_eq!(describe_delay(Duration::from_secs(seconds)), printed);
            assert_eq!(
                parse_delay(printed).unwrap(),
                Some(Duration::from_secs(seconds))
            );
        }
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
        for manifest in [None, Some(manifest(None, None))] {
            assert_eq!(
                cadence(None, manifest.as_ref()).unwrap(),
                Cadence {
                    delay: Some(DEFAULT_DELAY),
                    recheck: Recheck::Backoff,
                }
            );
        }
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

        for value in ["backoff", "BACKOFF", "10m", "1h"] {
            assert!(
                cadence(None, Some(&manifest(None, Some(value)))).is_ok(),
                "{value}"
            );
        }
    }
}
