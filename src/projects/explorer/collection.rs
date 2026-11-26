use std::{
    sync::mpsc::{self, Receiver},
    thread,
    time::Instant,
};

use anyhow::Result;

use super::{Explorer, Mode, REFRESH, tree::Project};
use crate::projects::snapshot::collect;

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
                let cursor = self.tree.cursor();
                self.tree.replace(projects);
                // A row shift can put a different item next to the menu.
                if let Mode::Menu(menu) = &self.mode
                    && (cursor != self.tree.cursor()
                        || !self.tree.selected().is_some_and(|item| menu.matches(item)))
                {
                    self.mode = Mode::Browse;
                }
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
