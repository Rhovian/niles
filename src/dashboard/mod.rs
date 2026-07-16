mod render;
mod rows;
mod snapshot;
#[cfg(test)]
mod snapshot_tests;
mod status;
mod usage_cell;
mod usage_throttle;

use std::{
    io::{self, Write},
    thread,
    time::Duration,
};

use anyhow::Result;
use camino::Utf8Path;
use chrono::Utc;

const CLEAR_AND_HOME: &str = "\x1B[2J\x1B[H";
const TICK: Duration = Duration::from_secs(1);
const USAGE_RECOMPUTE_TICKS: u64 = 5;

pub(crate) fn run(workspace: &Utf8Path) -> Result<()> {
    let mut usage_cache = usage_cell::DashboardUsageCache::default();
    let mut usage_throttle = usage_throttle::UsageRefreshThrottle::new(USAGE_RECOMPUTE_TICKS);
    loop {
        let snapshot = snapshot::collect(
            workspace,
            Utc::now(),
            &mut usage_cache,
            usage_throttle.should_recompute(),
        );
        let mut stdout = io::stdout();
        write!(stdout, "{CLEAR_AND_HOME}{}", render::render(&snapshot))?;
        stdout.flush()?;
        thread::sleep(TICK);
    }
}
