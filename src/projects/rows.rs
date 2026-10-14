use anyhow::Result;
use chrono::{DateTime, Utc};

use super::registry::Entry;
use crate::{
    session,
    telemetry::{SessionState, Usage},
    tmux, worker,
};

pub(super) struct Row {
    pub entry: Entry,
    pub state: State,
}

pub(super) enum State {
    Missing,
    NotRunning,
    Running,
    Starting,
    Waiting(Option<DateTime<Utc>>),
}

pub(super) fn collect(entries: Vec<Entry>) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    for entry in entries {
        let session = entry.name.session()?;
        let state = if !entry.path.is_dir() {
            State::Missing
        } else if tmux::project_session(&session)?.as_deref() != Some(entry.path.as_str())
            || !tmux::lead_running(&session)?
        {
            State::NotRunning
        } else {
            let lead = session::latest_lead(&entry.path)?;
            let record = lead
                .as_ref()
                .map(|meta| worker::lead_session(meta, Utc::now()))
                .transpose()?;
            let state = record
                .as_ref()
                .map_or(SessionState::Unknown, |record| record.state);
            let usage = record.and_then(|record| record.usage);
            let workers_working = worker::status_workers(&entry.path)?
                .iter()
                .any(|worker| worker.state == SessionState::Working);
            project_state(state, usage.as_ref(), workers_working)
        };
        rows.push(Row { entry, state });
    }
    sort_rows(&mut rows);
    Ok(rows)
}

fn sort_rows(rows: &mut [Row]) {
    rows.sort_by_key(|row| match row.state {
        State::Waiting(Some(since)) => (0, Some(since)),
        State::Waiting(None) => (1, None),
        State::Missing | State::NotRunning | State::Running | State::Starting => (2, None),
    });
}

fn project_state(state: SessionState, usage: Option<&Usage>, workers_working: bool) -> State {
    if workers_working || state == SessionState::Working {
        State::Running
    } else if state == SessionState::Starting {
        State::Starting
    } else {
        State::Waiting(usage.and_then(|usage| usage.last_turn_at))
    }
}

pub(crate) fn abbreviate(tokens: u64) -> String {
    if tokens < 1_000 {
        return tokens.to_string();
    }
    let mut scaled = tokens as f64 / 1_000.0;
    let mut suffix = "k";
    // Climbs while the value would print as 1000 or more, so 999_600 is 1.0M, not 1000k.
    for larger in ["M", "B", "T"] {
        if scaled < 999.5 {
            break;
        }
        scaled /= 1_000.0;
        suffix = larger;
    }
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
        assert_eq!(abbreviate(999_600), "1.0M");
        assert_eq!(abbreviate(2_000_000_000), "2.0B");
        assert_eq!(abbreviate(45_000_000_000_000), "45T");
        assert_eq!(abbreviate(u64::MAX), "18446744T");
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
    fn project_activity_requires_a_known_working_agent() {
        let mut usage = Usage {
            input_tokens: 1,
            output_tokens: 2,
            cache_read_tokens: 3,
            cache_write_tokens: None,
            reasoning_tokens: None,
            last_turn_at: None,
            estimated_cost_usd: None,
            buckets: Default::default(),
            prompt_tokens: None,
            context_window: None,
        };
        for lead_state in [
            SessionState::Waiting,
            SessionState::Working,
            SessionState::Unknown,
        ] {
            for workers_working in [false, true] {
                let active = workers_working || lead_state == SessionState::Working;
                assert_eq!(
                    matches!(
                        project_state(lead_state, Some(&usage), workers_working),
                        State::Running
                    ),
                    active,
                    "lead={lead_state:?}, workers_working={workers_working}",
                );
            }
        }
        assert!(matches!(
            project_state(SessionState::Unknown, None, false),
            State::Waiting(None)
        ));
        assert!(matches!(
            project_state(SessionState::Unknown, None, true),
            State::Running
        ));
        usage.last_turn_at = Some(Utc::now());
        assert!(matches!(
            project_state(SessionState::Waiting, Some(&usage), false),
            State::Waiting(Some(since)) if Some(since) == usage.last_turn_at
        ));
    }
}
