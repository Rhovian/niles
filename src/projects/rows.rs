use anyhow::Result;
use chrono::{DateTime, Utc};

use super::registry::Entry;
use crate::{
    session, store,
    telemetry::{SessionState, Usage},
    tmux, worker,
};

pub(super) struct Row {
    pub entry: Entry,
    pub state: State,
    pub lead_tokens: Option<u64>,
    pub workers: usize,
}

pub(super) enum State {
    Missing,
    NotRunning,
    Running,
    Waiting(Option<DateTime<Utc>>),
}

pub(super) fn collect(entries: Vec<Entry>) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    for entry in entries {
        let workers = store::worker_locations(&entry.path)?.len();
        let session = entry.name.session()?;
        let (state, lead_tokens) = if !entry.path.is_dir() {
            (State::Missing, None)
        } else if tmux::project_session(&session)?.as_deref() != Some(entry.path.as_str())
            || !tmux::lead_running(&session)?
        {
            (State::NotRunning, None)
        } else {
            let usage = session::latest_lead(&entry.path)?
                .map(|meta| worker::lead_usage(&meta))
                .transpose()?
                .flatten();
            match usage {
                Some(usage) => (lead_state(&usage), Some(usage.total_tokens())),
                None => (State::Running, None),
            }
        };
        rows.push(Row {
            entry,
            state,
            lead_tokens,
            workers,
        });
    }
    sort_rows(&mut rows);
    Ok(rows)
}

fn sort_rows(rows: &mut [Row]) {
    rows.sort_by_key(|row| match row.state {
        State::Waiting(Some(since)) => (0, Some(since)),
        State::Waiting(None) => (1, None),
        State::Missing | State::NotRunning | State::Running => (2, None),
    });
}

fn lead_state(usage: &Usage) -> State {
    match usage.state {
        Some(SessionState::Waiting) => State::Waiting(usage.last_turn_at),
        Some(SessionState::Working) | None => State::Running,
    }
}

pub(crate) fn abbreviate(tokens: u64) -> String {
    if tokens < 1_000 {
        return tokens.to_string();
    }
    let (scaled, suffix) = if tokens < 1_000_000 {
        (tokens as f64 / 1_000.0, "k")
    } else {
        (tokens as f64 / 1_000_000.0, "M")
    };
    if scaled >= 10.0 {
        format!("{scaled:.0}{suffix}")
    } else {
        format!("{scaled:.1}{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_scale() {
        assert_eq!(abbreviate(88_000), "88k");
        assert_eq!(abbreviate(1_200_000), "1.2M");
    }
    #[test]
    fn waiting_sorts_oldest_first() {
        let entry = |name| Entry {
            name: super::super::registry::ProjectName::parse(name).unwrap(),
            path: "/tmp".into(),
        };
        let row = |name, state| Row {
            entry: entry(name),
            state,
            lead_tokens: None,
            workers: 0,
        };
        let now = Utc::now();
        let mut rows = [
            row("a", State::Running),
            row("b", State::Waiting(Some(now))),
            row("d", State::Waiting(None)),
            row(
                "c",
                State::Waiting(Some(now - chrono::Duration::minutes(2))),
            ),
        ];
        sort_rows(&mut rows);
        assert_eq!(
            rows.map(|row| row.entry.name.as_str().to_owned()),
            ["c", "b", "d", "a"]
        );
    }
    #[test]
    fn waiting_without_turn_time_stays_waiting() {
        let usage = Usage {
            input_tokens: 1,
            output_tokens: 2,
            cache_read_tokens: 3,
            cache_write_tokens: None,
            reasoning_tokens: None,
            last_turn_at: None,
            state: Some(SessionState::Waiting),
            estimated_cost_usd: None,
        };
        assert!(matches!(lead_state(&usage), State::Waiting(None)));
    }
}
