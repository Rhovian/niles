use std::collections::VecDeque;

use anyhow::{Context, Result};
use axum::response::sse::Event;
use serde::Serialize;

use super::{Session, Source, resolve};
use crate::{
    projects::status::ForegroundSession,
    telemetry::transcript::{TextBlock, TextRole, TextTurn, TranscriptReader},
};

const RECENT_TURNS: usize = 200;

#[derive(Debug, Serialize)]
pub(super) struct Turn {
    id: String,
    role: &'static str,
    text: String,
}

#[derive(Serialize)]
pub(super) struct Snapshot {
    turns: VecDeque<Turn>,
    truncated: bool,
}

pub(super) struct Feed {
    record: ForegroundSession,
    view: Session,
    reader: Option<TranscriptReader>,
    next: u64,
    status_sent: bool,
}

impl Feed {
    // axum's event stream has no transcript retention or reconnect snapshot policy.
    // https://docs.rs/axum/0.8.9/axum/response/struct.Sse.html
    pub(super) fn open(record: ForegroundSession) -> Result<(Self, Snapshot)> {
        let mut feed = Self {
            view: Session::from_record(&record),
            record,
            reader: None,
            next: 0,
            status_sent: false,
        };
        let mut snapshot = Snapshot {
            turns: VecDeque::new(),
            truncated: false,
        };
        for turn in feed.read()? {
            snapshot.turns.push_back(turn);
            if snapshot.turns.len() > RECENT_TURNS {
                snapshot.turns.pop_front();
                snapshot.truncated = true;
            }
        }
        Ok((feed, snapshot))
    }

    fn read(&mut self) -> Result<Vec<Turn>> {
        if self.reader.is_none()
            && let Some(link) = &self.record.meta.session_link
        {
            self.reader = TranscriptReader::open(
                link,
                &self.record.meta.workspace,
                self.record
                    .meta
                    .brief
                    .parent()
                    .context("session brief has no parent")?,
                self.record.meta.created_at,
            )?;
        }
        let turns = match &mut self.reader {
            Some(reader) => reader.read()?,
            None => Vec::new(),
        };
        Ok(turns
            .into_iter()
            .map(|turn| {
                let id = format!("{}:{}", self.view.id, self.next);
                self.next += 1;
                let role = match turn.role {
                    TextRole::User => "user",
                    TextRole::Assistant => "agent",
                };
                Turn {
                    id,
                    role,
                    text: render(turn),
                }
            })
            .collect())
    }

    pub(super) fn poll(&mut self, source: &Source) -> Result<Option<Vec<Event>>> {
        let Some(record) = resolve(source, &self.view.id)? else {
            return Ok(None);
        };
        let view = Session::from_record(&record);
        let mut events = Vec::new();
        if !self.status_sent || view != self.view {
            events.push(Event::default().event("status").json_data(&view)?);
            self.status_sent = true;
            self.view = view;
        }
        for turn in self.read()? {
            events.push(Event::default().event("turn").json_data(turn)?);
        }
        Ok(Some(events))
    }
}

// serde_json exposes fields, not provider tool summaries for the text-only API v1.
// https://docs.rs/serde_json/1.0.151/serde_json/enum.Value.html
pub(super) fn render(turn: TextTurn) -> String {
    match turn.content {
        TextBlock::Text { text } => text,
        TextBlock::ToolUse { name, input } => {
            let summary = ["command", "cmd", "description", "path"]
                .into_iter()
                .find_map(|key| input.get(key).and_then(serde_json::Value::as_str))
                .or_else(|| input.as_str())
                .and_then(|summary| summary.lines().next());
            match summary {
                Some(summary) => format!("▸ {name}: {summary}"),
                None => format!("▸ {name}"),
            }
        }
    }
}
