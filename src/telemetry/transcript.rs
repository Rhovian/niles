use super::SessionLink;
use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
};

mod project;
use project::file_turns;

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
    file: BufReader<File>,
    family: Family,
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
        };
        path.map(|path| {
            Ok(Self {
                file: BufReader::new(
                    File::open(&path).with_context(|| format!("open transcript {path}"))?,
                ),
                family,
            })
        })
        .transpose()
    }

    pub(crate) fn read(&mut self) -> Result<Vec<TextTurn>> {
        read_jsonl(&mut self.file, self.family)
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

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
