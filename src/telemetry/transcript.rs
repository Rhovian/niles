use super::SessionLink;
use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
};

mod project;
use project::{file_turns, message_turns};

#[cfg(feature = "remote")]
pub(crate) struct TranscriptSource {
    pub link: Option<SessionLink>,
    pub workspace: camino::Utf8PathBuf,
    pub needle: camino::Utf8PathBuf,
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "remote")]
impl TranscriptSource {
    pub fn open(&self) -> Result<Option<TranscriptReader>> {
        match &self.link {
            Some(link) => {
                TranscriptReader::open(link, &self.workspace, &self.needle, self.created_at)
            }
            None => Ok(None),
        }
    }
}

/// One visible text/tool block, in persisted order. Identical blocks remain distinct.
#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct TextTurn {
    pub(crate) role: TextRole,
    pub(crate) timestamp: Option<DateTime<Utc>>,
    pub(crate) content: TextBlock,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TextRole {
    User,
    Assistant,
}

impl TextRole {
    fn parse(role: &str) -> Option<Self> {
        match role {
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum TextBlock {
    Text { text: String },
    ToolUse { name: String, input: Value },
}

#[derive(Clone, Copy)]
pub(super) enum Family {
    Claude,
    Codex,
    Pi,
}

/// Independent of usage/activity reads. Owns an append cursor, not a polling timer.
pub(crate) struct TranscriptReader {
    source: Source,
}

enum Source {
    Jsonl {
        file: BufReader<File>,
        family: Family,
    },
    Hermes {
        db: Connection,
        session: String,
        after: i64,
    },
}

impl TranscriptReader {
    pub(crate) fn open(
        link: &SessionLink,
        workspace: &Utf8Path,
        needle: &Utf8Path,
        created_at: DateTime<Utc>,
    ) -> Result<Option<Self>> {
        Self::open_in(
            &crate::util::home_dir()?,
            link,
            workspace,
            needle,
            created_at,
        )
    }

    fn open_in(
        home: &Utf8Path,
        link: &SessionLink,
        workspace: &Utf8Path,
        needle: &Utf8Path,
        created_at: DateTime<Utc>,
    ) -> Result<Option<Self>> {
        let (path, family) = match link {
            SessionLink::Claude { session_id } => (
                super::claude::transcript_path(home, session_id)?.map(|(_, path)| path),
                Family::Claude,
            ),
            SessionLink::Codex => (
                super::codex::read_codex_source(home, workspace, needle, created_at)?
                    .map(|(path, _)| path),
                Family::Codex,
            ),
            SessionLink::Pi { session_dir } => {
                (super::pi::transcript_path(session_dir)?, Family::Pi)
            }
            SessionLink::Hermes { source } => {
                return Self::hermes(super::hermes::open_db(home)?, source);
            }
        };
        path.map(|path| {
            Ok(Self {
                source: Source::Jsonl {
                    file: BufReader::new(
                        File::open(&path).with_context(|| format!("open transcript {path}"))?,
                    ),
                    family,
                },
            })
        })
        .transpose()
    }

    fn hermes(db: Connection, source: &str) -> Result<Option<Self>> {
        let session = db
            .query_row(
                "SELECT id FROM sessions WHERE source = ?1
             AND json_extract(model_config, '$._delegate_from') IS NULL
             ORDER BY started_at DESC LIMIT 1",
                [source],
                |row| row.get(0),
            )
            .optional()?;
        Ok(session.map(|session| Self {
            source: Source::Hermes {
                db,
                session,
                after: 0,
            },
        }))
    }

    pub(crate) fn read(&mut self) -> Result<Vec<TextTurn>> {
        match &mut self.source {
            Source::Jsonl { file, family } => read_jsonl(file, *family),
            Source::Hermes { db, session, after } => read_hermes(db, session, after),
        }
    }
}

// std BufRead::read_until includes delimiters but doesn't distinguish committed JSONL
// records from a partial append: https://doc.rust-lang.org/1.89.0/std/io/trait.BufRead.html#method.read_until
fn read_jsonl(file: &mut BufReader<File>, family: Family) -> Result<Vec<TextTurn>> {
    let start = file.stream_position()?;
    let result = (|| {
        let mut turns = Vec::new();
        let mut line = Vec::new();
        loop {
            let position = file.stream_position()?;
            line.clear();
            file.read_until(b'\n', &mut line)?;
            if !line.ends_with(b"\n") {
                file.seek(SeekFrom::Start(position))?;
                break;
            }
            turns.extend(file_turns(family, serde_json::from_slice(&line)?)?);
        }
        Ok(turns)
    })();
    if result.is_err() {
        file.seek(SeekFrom::Start(start))?;
    }
    result
}

// Hermes 71d34bad hermes_state_messages.py persists IDs, active flags, sentinel-prefixed
// content and OpenAI tool_calls. rusqlite provides row iteration, not a message tail:
// https://docs.rs/rusqlite/0.40.2/rusqlite/struct.Statement.html
fn read_hermes(db: &Connection, session: &str, after: &mut i64) -> Result<Vec<TextTurn>> {
    let mut statement = db.prepare(
        "SELECT id, role, content, tool_calls, timestamp FROM messages
         WHERE session_id = ?1 AND active = 1 AND id > ?2 ORDER BY id",
    )?;
    let mut rows = statement.query(rusqlite::params![session, *after])?;
    let mut turns = Vec::new();
    let mut next = *after;
    while let Some(row) = rows.next()? {
        next = row.get("id")?;
        let role: String = row.get("role")?;
        let Some(role) = TextRole::parse(&role) else {
            continue;
        };
        let content: Option<String> = row.get("content")?;
        let content = match content {
            Some(content) => match content.strip_prefix("\0json:") {
                Some(json) => serde_json::from_str(json)?,
                None => Value::String(content),
            },
            None => Value::Null,
        };
        let seconds: f64 = row.get("timestamp")?;
        let at = DateTime::from_timestamp_micros((seconds * 1_000_000.0).round() as i64)
            .context("Hermes message timestamp out of range")?;
        turns.extend(message_turns(role, Some(at), content));
        let calls: Option<String> = row.get("tool_calls")?;
        if let Some(calls) = calls {
            let calls: Vec<Value> = serde_json::from_str(&calls)?;
            for mut call in calls {
                if let Some(function) = call.get_mut("function") {
                    turns.extend(project::function_turn(function.take(), Some(at)));
                }
            }
        }
    }
    *after = next;
    Ok(turns)
}

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
