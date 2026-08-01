//! Decides whether a waiter registration is still a live claim on the worker.
//!
//! Three questions live here, and nothing else: is the recorded pid structurally usable, has the
//! heartbeat gone stale, and how does a registration written before heartbeats existed age out.

use chrono::{DateTime, Utc};

use super::super::outcome::HeartbeatState;
use super::WaiterRegistration;

const MIN_STALE_HEARTBEAT_MS: u64 = 15_000;

/// Upper bound applied to a registration's own `heartbeat_interval_ms` before it is used in the
/// staleness threshold. Without it a corrupt or absurd value saturates the threshold and disables
/// stale detection entirely, which is the failure #102 exists to prevent. Matches the largest
/// interval `wait` accepts, so a live waiter is never falsely declared stale.
const MAX_HEARTBEAT_INTERVAL_MS: u64 = 3_600_000;

/// A registration with no heartbeat fields predates heartbeats, so it can never be proven fresh.
/// It ages out from `started_at` instead, once it has outlived the default wait timeout by which
/// any legitimate waiter would have exited on its own.
const MISSING_HEARTBEAT_MAX_AGE_MS: u64 = 3_600_000;

/// Reason string for a pid that cannot belong to a live waiter, or `None` if the pid is usable.
///
/// Both cases are structurally impossible for a real holder, so callers reclaim rather than treat
/// the registration as a conflict. Pid 0 matters most: `kill(0, sig)` signals the caller's entire
/// process group, so it must never reach the takeover path.
pub(super) fn invalid_pid_reason(pid: u32) -> Option<&'static str> {
    if pid == 0 {
        return Some("pid-zero");
    }
    if pid == std::process::id() {
        return Some("pid-self");
    }
    None
}

pub(super) fn heartbeat_state(waiter: &WaiterRegistration, now: DateTime<Utc>) -> HeartbeatState {
    let (Some(heartbeat_at), Some(interval_ms)) =
        (waiter.heartbeat_at, waiter.heartbeat_interval_ms)
    else {
        return if age_ms(waiter.started_at, now) > saturating_i64(MISSING_HEARTBEAT_MAX_AGE_MS) {
            HeartbeatState::Stale
        } else {
            HeartbeatState::Missing
        };
    };

    let threshold = interval_ms
        .min(MAX_HEARTBEAT_INTERVAL_MS)
        .saturating_mul(3)
        .max(MIN_STALE_HEARTBEAT_MS);
    if age_ms(heartbeat_at, now) > saturating_i64(threshold) {
        HeartbeatState::Stale
    } else {
        HeartbeatState::Fresh
    }
}

fn age_ms(since: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    now.signed_duration_since(since).num_milliseconds()
}

fn saturating_i64(value: u64) -> i64 {
    if value > i64::MAX as u64 {
        i64::MAX
    } else {
        value as i64
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;

    fn registration(
        heartbeat_at: Option<DateTime<Utc>>,
        heartbeat_interval_ms: Option<u64>,
        started_at: DateTime<Utc>,
    ) -> WaiterRegistration {
        WaiterRegistration {
            pid: 1234,
            started_at,
            token: "token".to_owned(),
            heartbeat_at,
            heartbeat_interval_ms,
        }
    }

    #[test]
    fn pid_zero_and_self_are_never_usable() {
        assert_eq!(invalid_pid_reason(0), Some("pid-zero"));
        assert_eq!(
            invalid_pid_reason(std::process::id()),
            Some("pid-self"),
            "a registration holding our own pid is a recycled leftover, not a live holder"
        );
        assert_eq!(invalid_pid_reason(std::process::id().wrapping_add(1)), None);
    }

    #[test]
    fn saturating_heartbeat_interval_cannot_disable_stale_detection() {
        let now = Utc::now();
        let waiter = registration(
            Some(now - TimeDelta::days(365)),
            Some(u64::MAX),
            now - TimeDelta::days(365),
        );

        assert_eq!(
            heartbeat_state(&waiter, now),
            HeartbeatState::Stale,
            "u64::MAX must be clamped, not saturated into an unreachable threshold"
        );
    }

    #[test]
    fn large_but_accepted_interval_still_reports_fresh() {
        let now = Utc::now();
        let waiter = registration(
            Some(now - TimeDelta::seconds(60)),
            Some(MAX_HEARTBEAT_INTERVAL_MS),
            now,
        );

        assert_eq!(
            heartbeat_state(&waiter, now),
            HeartbeatState::Fresh,
            "clamping must not declare a waiter stale inside its own poll interval"
        );
    }

    #[test]
    fn missing_heartbeat_ages_out_from_started_at() {
        let now = Utc::now();
        let recent = registration(None, None, now - TimeDelta::minutes(5));
        let ancient = registration(None, None, now - TimeDelta::days(1));

        assert_eq!(heartbeat_state(&recent, now), HeartbeatState::Missing);
        assert_eq!(
            heartbeat_state(&ancient, now),
            HeartbeatState::Stale,
            "a pre-heartbeat registration must not hold a worker forever"
        );
    }

    #[test]
    fn fresh_heartbeat_within_threshold_is_fresh() {
        let now = Utc::now();
        let waiter = registration(Some(now - TimeDelta::seconds(1)), Some(1000), now);

        assert_eq!(heartbeat_state(&waiter, now), HeartbeatState::Fresh);
    }
}
