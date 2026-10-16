use std::{
    sync::mpsc::{self, Receiver},
    thread,
    time::Instant,
};

use anyhow::Result;
use chrono::Utc;

use super::{Explorer, REFRESH, tree::Project};
use crate::projects::{
    registry,
    rows::{self, State},
    windows,
};

impl Explorer {
    /// Collects in place, for an action whose next step reads the result.
    pub(super) fn refresh(&mut self) {
        let started = Instant::now();
        self.accept_collection(started, collect());
    }

    pub(super) fn accept_collection(&mut self, started: Instant, projects: Result<Vec<Project>>) {
        if started <= self.collected {
            return;
        }
        self.collected = started;
        // ratatui 0.30.2 Terminal::draw renders application-owned state; it cannot retain refreshes:
        // https://docs.rs/ratatui/0.30.2/ratatui/struct.Terminal.html#method.draw
        // Keep the whole last successful snapshot rather than present partial data as current.
        match projects {
            Ok(projects) => {
                self.tree.replace(projects);
                self.collection_error = None;
            }
            Err(error) => {
                self.collection_error = Some(format!("refresh failed; retrying: {error:#}"))
            }
        }
    }
}

/// Collects every `REFRESH` on its own thread, so a slow collect never stalls drawing. Each result
/// carries when its collect began.
pub(super) fn collector() -> Receiver<(Instant, Result<Vec<Project>>)> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        loop {
            thread::sleep(REFRESH);
            let started = Instant::now();
            if sender.send((started, collect())).is_err() {
                break;
            }
        }
    });
    receiver
}

fn collect() -> Result<Vec<Project>> {
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
