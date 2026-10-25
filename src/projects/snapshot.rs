use anyhow::Result;
use chrono::Utc;

use super::{
    registry,
    rows::{self, Row, State},
    windows::{self, SessionAgents},
};

pub(super) struct Project {
    pub row: Row,
    /// Present exactly when the project's lead is running.
    pub agents: Option<SessionAgents>,
}

pub(super) fn collect() -> Result<Vec<Project>> {
    let now = Utc::now();
    rows::collect(registry::entries()?)?
        .into_iter()
        .map(|mut row| {
            let agents = match row.state {
                State::Running | State::Starting | State::Waiting(_) => {
                    windows::session_agents(&row.entry.name.session()?, Some(&row.entry.path), now)?
                }
                State::Missing | State::NotRunning => None,
            };
            if agents.is_none() && !matches!(row.state, State::Missing) {
                row.state = State::NotRunning;
            }
            Ok(Project { row, agents })
        })
        .collect()
}
