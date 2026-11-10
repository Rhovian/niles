use anyhow::Result;
use chrono::{DateTime, DurationRound, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The width of a bucket. Every real UTC offset is a multiple of it, so a bucket never straddles
/// local midnight.
pub(crate) const BUCKET: TimeDelta = TimeDelta::minutes(15);

/// Tokens spent per UTC-aligned `BUCKET`, keyed by the bucket's start.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Buckets(BTreeMap<DateTime<Utc>, u64>);

impl Buckets {
    pub(crate) fn add(&mut self, at: DateTime<Utc>, tokens: u64) -> Result<()> {
        let total = self.0.entry(at.duration_trunc(BUCKET)?).or_default();
        *total = super::sum([*total, tokens])?;
        Ok(())
    }

    #[cfg(feature = "remote")]
    pub(crate) fn since(mut self, start: DateTime<Utc>) -> Self {
        Self(self.0.split_off(&start))
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (DateTime<Utc>, u64)> + '_ {
        self.0.iter().map(|(start, tokens)| (*start, *tokens))
    }
}
