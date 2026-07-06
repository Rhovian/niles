#[derive(Debug, Clone)]
pub(crate) struct UsageRefreshThrottle {
    interval_ticks: u64,
    tick: u64,
}

impl UsageRefreshThrottle {
    pub(crate) fn new(interval_ticks: u64) -> Self {
        Self {
            interval_ticks: interval_ticks.max(1),
            tick: 0,
        }
    }

    pub(crate) fn should_recompute(&mut self) -> bool {
        let recompute = self.tick == 0 || self.tick.is_multiple_of(self.interval_ticks);
        self.tick = self.tick.saturating_add(1);
        recompute
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recomputes_first_tick_and_every_interval_after_reuse_ticks() {
        let mut throttle = UsageRefreshThrottle::new(5);
        let decisions = (0..7)
            .map(|_| throttle.should_recompute())
            .collect::<Vec<_>>();

        assert_eq!(
            decisions,
            vec![true, false, false, false, false, true, false]
        );
    }
}
