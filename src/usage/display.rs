use std::fmt;

use super::snapshot::{UsageSnapshot, UsageSnapshotUsage, UsageTurnCounts};

const MISSING: &str = "-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UsageDisplayStatus {
    Available,
    Unavailable,
    Pending,
}

impl fmt::Display for UsageDisplayStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Available => "available",
            Self::Unavailable => "unavailable",
            Self::Pending => "pending",
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct UsageTotals {
    pub(crate) total: Option<u64>,
    pub(crate) input: Option<u64>,
    pub(crate) cache_create: Option<u64>,
    pub(crate) cache_read: Option<u64>,
    pub(crate) cached: Option<u64>,
    pub(crate) output: Option<u64>,
    pub(crate) reasoning: Option<u64>,
}

impl UsageTotals {
    fn add_assign(&mut self, other: &Self) {
        add_column(&mut self.total, other.total);
        add_column(&mut self.input, other.input);
        add_column(&mut self.cache_create, other.cache_create);
        add_column(&mut self.cache_read, other.cache_read);
        add_column(&mut self.cached, other.cached);
        add_column(&mut self.output, other.output);
        add_column(&mut self.reasoning, other.reasoning);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UsageDisplay {
    pub(crate) status: UsageDisplayStatus,
    pub(crate) wall_seconds: Option<i64>,
    pub(crate) turns: Option<u64>,
    pub(crate) totals: UsageTotals,
}

impl UsageDisplay {
    pub(crate) fn from_snapshot(snapshot: &UsageSnapshot, missing_is_pending: bool) -> Self {
        match &snapshot.usage {
            UsageSnapshotUsage::Available {
                input_tokens,
                cached_input_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
                output_tokens,
                reasoning_output_tokens,
                total_tokens,
                ..
            } => Self {
                status: UsageDisplayStatus::Available,
                wall_seconds: snapshot.wall_seconds,
                turns: display_turns(&snapshot.turns),
                totals: UsageTotals {
                    total: Some(*total_tokens),
                    input: Some(*input_tokens),
                    cache_create: *cache_creation_input_tokens,
                    cache_read: *cache_read_input_tokens,
                    cached: *cached_input_tokens,
                    output: Some(*output_tokens),
                    reasoning: *reasoning_output_tokens,
                },
            },
            UsageSnapshotUsage::Unavailable { reason, .. } => {
                let status = if missing_is_pending
                    && matches!(reason, super::snapshot::UsageUnavailableReason::Missing)
                {
                    UsageDisplayStatus::Pending
                } else {
                    UsageDisplayStatus::Unavailable
                };
                Self {
                    status,
                    wall_seconds: snapshot.wall_seconds,
                    turns: None,
                    totals: UsageTotals::default(),
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn pending(wall_seconds: Option<i64>) -> Self {
        Self {
            status: UsageDisplayStatus::Pending,
            wall_seconds,
            turns: None,
            totals: UsageTotals::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn unavailable(wall_seconds: Option<i64>) -> Self {
        Self {
            status: UsageDisplayStatus::Unavailable,
            wall_seconds,
            turns: None,
            totals: UsageTotals::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct UsageRollup {
    pub(crate) available_subjects: u64,
    pub(crate) wall_seconds: Option<u64>,
    pub(crate) totals: UsageTotals,
}

impl UsageRollup {
    pub(crate) fn add(&mut self, usage: &UsageDisplay) {
        if let Some(seconds) = usage.wall_seconds
            && let Ok(seconds) = u64::try_from(seconds)
        {
            self.wall_seconds = Some(match self.wall_seconds {
                Some(current) => current.saturating_add(seconds),
                None => seconds,
            });
        }

        if matches!(usage.status, UsageDisplayStatus::Available) {
            self.available_subjects = self.available_subjects.saturating_add(1);
            self.totals.add_assign(&usage.totals);
        }
    }
}

pub(crate) fn format_optional(value: Option<u64>) -> String {
    match value {
        Some(value) => value.to_string(),
        None => MISSING.to_owned(),
    }
}

pub(crate) fn format_wall(value: Option<i64>) -> String {
    match value {
        Some(value) if value >= 0 => format!("{value}s"),
        Some(_) | None => MISSING.to_owned(),
    }
}

pub(crate) fn format_rollup_wall(value: Option<u64>) -> String {
    match value {
        Some(value) => format!("{value}s"),
        None => MISSING.to_owned(),
    }
}

fn add_column(target: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *target = Some(match *target {
            Some(current) => current.saturating_add(value),
            None => value,
        });
    }
}

fn display_turns(turns: &UsageTurnCounts) -> Option<u64> {
    if let Some(count) = turns.transcript_user_messages {
        return Some(count);
    }
    if let Some(count) = turns.niles_prompt_count {
        return Some(count);
    }
    turns.usage_messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_family_rollup_sums_union_columns() {
        let codex = UsageDisplay {
            status: UsageDisplayStatus::Available,
            wall_seconds: Some(10),
            turns: Some(1),
            totals: UsageTotals {
                total: Some(18),
                input: Some(10),
                cache_create: None,
                cache_read: None,
                cached: Some(3),
                output: Some(5),
                reasoning: Some(2),
            },
        };
        let claude = UsageDisplay {
            status: UsageDisplayStatus::Available,
            wall_seconds: Some(20),
            turns: Some(1),
            totals: UsageTotals {
                total: Some(30),
                input: Some(7),
                cache_create: Some(8),
                cache_read: Some(9),
                cached: None,
                output: Some(6),
                reasoning: None,
            },
        };
        let pending = UsageDisplay::pending(Some(5));

        let mut rollup = UsageRollup::default();
        rollup.add(&codex);
        rollup.add(&claude);
        rollup.add(&pending);

        assert_eq!(rollup.available_subjects, 2);
        assert_eq!(rollup.wall_seconds, Some(35));
        assert_eq!(
            rollup.totals,
            UsageTotals {
                total: Some(48),
                input: Some(17),
                cache_create: Some(8),
                cache_read: Some(9),
                cached: Some(3),
                output: Some(11),
                reasoning: Some(2),
            }
        );
    }
}
