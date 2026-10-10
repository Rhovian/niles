use std::collections::BTreeMap;

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

/// The width of a bucket. Every real UTC offset is a multiple of it, so a bucket never straddles
/// local midnight.
pub(crate) const BUCKET: TimeDelta = TimeDelta::minutes(15);

/// Tokens spent per UTC-aligned `BUCKET`, keyed by the bucket's start.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Buckets(BTreeMap<DateTime<Utc>, u64>);

impl Buckets {
    pub(crate) fn add(&mut self, at: DateTime<Utc>, tokens: u64) {
        let into = TimeDelta::seconds(at.timestamp().rem_euclid(BUCKET.num_seconds()))
            + TimeDelta::nanoseconds(at.timestamp_subsec_nanos().into());
        *self.0.entry(at - into).or_default() += tokens;
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (DateTime<Utc>, u64)> + '_ {
        self.0.iter().map(|(start, tokens)| (*start, *tokens))
    }
}
