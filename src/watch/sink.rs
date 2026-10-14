use super::{WatchLog, composer};
use crate::{
    tmux::{self, TmuxTarget},
    worker,
};
use anyhow::Result;
use chrono::{DateTime, Utc};

/// The edge: read worker panes, send lead nudges, and record diagnostics without using stdout.
pub(super) trait Sink {
    fn activity(
        &mut self,
        worker: &worker::WorkerSnapshot,
        now: DateTime<Utc>,
    ) -> Result<crate::telemetry::ResolvedSession> {
        worker::worker_session(worker, now)
    }
    fn nudge(&mut self, text: &str) -> Result<()>;
    fn has_draft(&mut self) -> bool;
    fn capture_visible(&mut self, target: &TmuxTarget) -> Result<String>;
    fn note(&mut self, line: &str);
}

pub(super) struct WatchSink {
    pub(super) target: TmuxTarget,
    pub(super) log: WatchLog,
    pub(super) composer: Option<&'static str>,
}

impl Sink for WatchSink {
    fn has_draft(&mut self) -> bool {
        let Some(marker) = self.composer else {
            return false;
        };
        let screen = tmux::capture_visible_rows(&self.target);
        let cursor = tmux::cursor_position(&self.target);
        match (screen, cursor) {
            (Ok(screen), Ok(cursor)) => composer::recognize(&screen, cursor, marker),
            (Err(err), _) | (_, Err(err)) => {
                self.note(&format!("could not read lead composer: {err:#}"));
                false
            }
        }
    }
    fn nudge(&mut self, text: &str) -> Result<()> {
        // Deliberately not `nudge_line` or a hand-rolled send-keys: `send_line` is the one place
        // that knows how to confirm a submit took, and a nudge that silently sat in the lead's
        // composer is the failure this whole feature exists to avoid.
        tmux::send_line(&self.target, text)
    }

    fn capture_visible(&mut self, target: &TmuxTarget) -> Result<String> {
        tmux::capture_visible_pane(target)
    }

    fn note(&mut self, line: &str) {
        self.log.note(line);
    }
}
