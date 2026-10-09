//! The TELEMETRY panel: tokens spent in a window, by project, by model and over time, with the
//! live sessions' context and the window's events.

use anyhow::{Context, Result};
use chrono::{DateTime, Days, Local, NaiveTime, TimeDelta, Utc};
use clap::ValueEnum;

use super::{lead_running, registry};
use crate::{telemetry::BUCKET, theme::Theme};

mod aggregate;
mod collect;
mod render;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Range {
    Today,
    #[value(name = "7d")]
    Week,
    #[value(name = "30d")]
    Month,
}

impl Range {
    pub(crate) const ALL: [Self; 3] = [Self::Today, Self::Week, Self::Month];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Week => "7 days",
            Self::Month => "30 days",
        }
    }

    /// Days back to the window's first local midnight, its bins' width, and how many there are.
    fn shape(self) -> (u64, TimeDelta, i32) {
        match self {
            Self::Today => (1, BUCKET, 96),
            Self::Week => (7, TimeDelta::hours(2), 84),
            Self::Month => (30, TimeDelta::hours(8), 90),
        }
    }
}

/// `[start, end)`: from local midnight `days - 1` days ago, for `count` bins. On a day the clocks
/// change, `end` is an hour off the next midnight; that is accepted.
#[derive(Clone, Copy, Debug)]
pub(super) struct Window {
    range: Range,
    start: DateTime<Utc>,
    bin: TimeDelta,
    count: i32,
}

impl Window {
    fn new(range: Range, now: DateTime<Local>) -> Result<Self> {
        let (days, bin, count) = range.shape();
        let date = now.date_naive() - Days::new(days - 1);
        let start = date
            .and_time(NaiveTime::MIN)
            .and_local_timezone(Local)
            .earliest()
            .with_context(|| format!("local midnight of {date} does not exist"))?;
        Ok(Self {
            range,
            start: start.to_utc(),
            bin,
            count,
        })
    }

    fn bin_start(&self, index: i32) -> DateTime<Utc> {
        self.start + self.bin * index
    }

    fn contains(&self, at: DateTime<Utc>) -> bool {
        self.start <= at && at < self.bin_start(self.count)
    }

    /// The bin a bucket starting at `bucket` falls in, if the window holds it.
    fn bin_of(&self, bucket: DateTime<Utc>) -> Option<usize> {
        if !self.contains(bucket) {
            return None;
        }
        // Not negative: the window holds the bucket.
        let offset = (bucket - self.start).num_seconds().cast_unsigned();
        // Local midnight is on a bucket boundary and bins are whole buckets, so no bucket
        // straddles two bins.
        debug_assert_eq!(offset % BUCKET.num_seconds().cast_unsigned(), 0);
        Some((offset / self.bin.num_seconds().cast_unsigned()) as usize)
    }
}

pub(super) fn panel(entries: &[registry::Entry], range: Range, theme: &Theme) -> Result<String> {
    let now = Local::now();
    // Closed sessions are always gathered for the longest window, so every range warms the same
    // caches.
    let since = Window::new(Range::Month, now)?.start;
    let projects = entries
        .iter()
        .map(|entry| collect::project(entry, lead_running(entry)?, since))
        .collect::<Result<Vec<_>>>()?;
    let (width, _) =
        ratatui::crossterm::terminal::size().context("failed to read the terminal size")?;
    let dashboard = aggregate::dashboard(&projects, Window::new(range, now)?);
    Ok(render::dashboard(&dashboard, theme, usize::from(width)))
}
