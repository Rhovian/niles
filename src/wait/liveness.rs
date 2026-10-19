use crate::worker::WaitHealth;
use std::time::{Duration, Instant};

const PROBE_INTERVAL: Duration = Duration::from_secs(5);
const IDLE_GRACE: Duration = Duration::from_secs(30);

// Rust 1.89 Instant (std/time.rs) measures elapsed time, but does not track idle
// observations or reset them when a new dispatch begins. Keep that state per worker.
#[derive(Default)]
pub(super) struct Liveness {
    checked_at: Option<Instant>,
    idle_since: Option<Instant>,
}

impl Liveness {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(super) fn due(&self, now: Instant) -> bool {
        self.checked_at
            .is_none_or(|last| now.duration_since(last) >= PROBE_INTERVAL)
    }

    pub(super) fn observe(
        &mut self,
        health: Option<WaitHealth>,
        now: Instant,
    ) -> Option<&'static str> {
        self.checked_at = Some(now);
        match health {
            Some(WaitHealth::Gone) => Some("worker window or agent exited without reporting"),
            Some(WaitHealth::Idle) => {
                let since = *self.idle_since.get_or_insert(now);
                (now.duration_since(since) >= IDLE_GRACE)
                    .then_some("worker idle for 30s without reporting")
            }
            Some(WaitHealth::Running | WaitHealth::Unknown) | None => {
                self.idle_since = None;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_dead_window_is_immediate_and_idle_requires_continuous_grace() {
        let now = Instant::now();
        let mut state = Liveness::default();
        assert!(state.due(now));
        assert!(state.observe(Some(WaitHealth::Gone), now).is_some());
        assert!(!state.due(now));
        assert!(state.due(now + PROBE_INTERVAL));
        assert!(state.observe(Some(WaitHealth::Idle), now).is_none());
        assert!(
            state
                .observe(
                    Some(WaitHealth::Idle),
                    now + IDLE_GRACE - Duration::from_secs(1)
                )
                .is_none()
        );
        assert!(
            state
                .observe(Some(WaitHealth::Idle), now + IDLE_GRACE)
                .is_some()
        );
        assert!(
            state
                .observe(Some(WaitHealth::Running), now + IDLE_GRACE)
                .is_none()
        );
        assert!(
            state
                .observe(Some(WaitHealth::Idle), now + IDLE_GRACE)
                .is_none()
        );
        assert!(
            state
                .observe(Some(WaitHealth::Unknown), now + IDLE_GRACE)
                .is_none()
        );
        assert!(
            state
                .observe(Some(WaitHealth::Idle), now + IDLE_GRACE)
                .is_none()
        );
        state.reset();
        assert!(state.due(now + IDLE_GRACE));
        assert!(
            state
                .observe(Some(WaitHealth::Idle), now + IDLE_GRACE)
                .is_none()
        );
    }
}
