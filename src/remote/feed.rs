use std::collections::VecDeque;

use anyhow::Result;
use axum::response::sse::Event;
use serde::Serialize;

use super::chat::{Chat, Lookup};
use crate::telemetry::transcript::{
    TextBlock, TextRole, TextTurn, TranscriptReader, TranscriptSource,
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
    chat: Chat,
    follower: Follower,
    next: u64,
    status_sent: bool,
}

impl Feed {
    // axum's event stream has no transcript retention or reconnect snapshot policy.
    // https://docs.rs/axum/0.8.9/axum/response/struct.Sse.html
    pub(super) fn open(chat: Chat) -> Result<(Self, Snapshot)> {
        let mut feed = Self {
            chat,
            follower: Follower::default(),
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
        let turns = self.follower.read(&self.chat.transcript)?;
        Ok(turns
            .into_iter()
            .filter_map(|turn| {
                let role = match turn.role {
                    TextRole::User | TextRole::Injected => "user",
                    TextRole::Assistant => "agent",
                };
                let text = mobile_text(turn)?;
                let id = format!("{}:{}", self.chat.id, self.next);
                self.next += 1;
                Some(Turn { id, role, text })
            })
            .collect())
    }

    pub(super) fn poll(&mut self, lookup: &Lookup) -> Result<Option<Vec<Event>>> {
        let Some(chat) = lookup()? else {
            return Ok(None);
        };
        let mut events = Vec::new();
        if let Some(status) = &chat.status
            && (!self.status_sent || chat.status != self.chat.status)
        {
            events.push(Event::default().event("status").json_data(status)?);
            self.status_sent = true;
        }
        self.chat = chat;
        for turn in self.read()? {
            events.push(Event::default().event("turn").json_data(turn)?);
        }
        Ok(Some(events))
    }
}

#[derive(Default)]
pub(super) struct Follower(Option<(TranscriptSource, TranscriptReader)>);
impl Follower {
    fn read(&mut self, source: &TranscriptSource) -> Result<Vec<TextTurn>> {
        // File handles keep their original source; worker IDs can be reused between polls.
        // https://doc.rust-lang.org/1.89.0/std/fs/struct.File.html
        if self.0.as_ref().is_none_or(|(current, _)| current != source) {
            self.0 = source.open()?.map(|reader| (source.clone(), reader));
        }
        match &mut self.0 {
            Some((_, reader)) => reader.read(),
            None => Ok(Vec::new()),
        }
    }
    pub(super) fn read_new(&mut self, source: &TranscriptSource) -> Result<Vec<TextTurn>> {
        let baseline = self.0.as_ref().is_none_or(|(current, _)| current != source);
        let turns = self.read(source)?;
        // A rollout discovered late is a baseline, not a batch of new messages.
        Ok(if baseline { Vec::new() } else { turns })
    }
}

pub(super) fn mobile_text(turn: TextTurn) -> Option<String> {
    let mut text = match turn.content {
        TextBlock::Text { text } => text,
        TextBlock::ToolUse { .. } => return None,
    };
    if matches!(turn.role, TextRole::Assistant) {
        // pulldown-cmark supplies complete code-block source ranges; remove them without
        // reserializing prose or implementing Markdown fence/indentation parsing ourselves.
        // https://docs.rs/pulldown-cmark/0.13.5/pulldown_cmark/struct.OffsetIter.html
        let blocks = pulldown_cmark::Parser::new_ext(&text, pulldown_cmark::Options::ENABLE_TABLES)
            .into_offset_iter()
            .filter_map(|(event, range)| {
                matches!(
                    event,
                    pulldown_cmark::Event::Start(pulldown_cmark::Tag::CodeBlock(_))
                )
                .then_some(range)
            })
            .collect::<Vec<_>>();
        for block in blocks.into_iter().rev() {
            text.replace_range(block, "");
        }
    }
    (!text.trim().is_empty()).then_some(text)
}
