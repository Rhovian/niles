use std::{collections::BTreeMap, fs};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value as JsonValue;

use super::snapshot::TranscriptTurnCounts;

const CLAUDE_PROJECTS_DIR: &str = "projects";

#[derive(Debug, Clone)]
pub(crate) struct ClaudeUsageSummary {
    pub(crate) usage: ClaudeTokenUsage,
    pub(crate) turns: TranscriptTurnCounts,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ClaudeTokenUsage {
    pub(crate) input_tokens: u64,
    pub(crate) cache_creation_input_tokens: u64,
    pub(crate) cache_read_input_tokens: u64,
    pub(crate) output_tokens: u64,
}

impl ClaudeTokenUsage {
    pub(crate) fn total_tokens(self) -> u64 {
        self.input_tokens
            + self.cache_creation_input_tokens
            + self.cache_read_input_tokens
            + self.output_tokens
    }
}

#[derive(Debug, Deserialize)]
struct ClaudeLine {
    #[serde(rename = "type")]
    kind: ClaudeLineKind,
    #[serde(rename = "sessionId")]
    _session_id: Option<String>,
    uuid: Option<String>,
    _cwd: Option<Utf8PathBuf>,
    _timestamp: Option<DateTime<Utc>>,
    message: Option<JsonValue>,
}

#[derive(Debug)]
enum ClaudeLineKind {
    User,
    Assistant,
    Other,
}

impl<'de> Deserialize<'de> for ClaudeLineKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "user" => Self::User,
            "assistant" => Self::Assistant,
            _ => Self::Other,
        })
    }
}

#[derive(Debug, Deserialize)]
struct ClaudeMessage {
    id: Option<String>,
    role: Option<ClaudeRole>,
    usage: Option<ClaudeUsage>,
}

#[derive(Debug)]
enum ClaudeRole {
    User,
    Assistant,
    Other,
}

impl<'de> Deserialize<'de> for ClaudeRole {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "user" => Self::User,
            "assistant" => Self::Assistant,
            _ => Self::Other,
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct ClaudeUsage {
    input_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
    output_tokens: u64,
}

pub(crate) fn transcript_path(
    claude_home: &Utf8Path,
    cwd: &Utf8Path,
    session_id: &str,
) -> Utf8PathBuf {
    claude_home
        .join(CLAUDE_PROJECTS_DIR)
        .join(cwd_project_slug(cwd))
        .join(format!("{session_id}.jsonl"))
}

pub(crate) fn cwd_project_slug(cwd: &Utf8Path) -> String {
    cwd.as_str()
        .chars()
        .map(|ch| if matches!(ch, '/' | '\\') { '-' } else { ch })
        .collect()
}

pub(crate) fn parse_claude_transcript_file(path: &Utf8Path) -> Result<Option<ClaudeUsageSummary>> {
    let body = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    parse_claude_transcript(&body)
        .with_context(|| format!("failed to parse Claude transcript {path}"))
}

pub(crate) fn parse_claude_transcript(body: &str) -> Result<Option<ClaudeUsageSummary>> {
    let mut usage_by_message = BTreeMap::<String, ClaudeUsage>::new();
    let mut turns = TranscriptTurnCounts::default();

    for (line_number, line) in parse_lines(body, "Claude transcript")? {
        let envelope: ClaudeLine = serde_json::from_value(line)
            .with_context(|| format!("invalid Claude transcript line {line_number}"))?;
        match envelope.kind {
            ClaudeLineKind::User => turns.transcript_user_messages += 1,
            ClaudeLineKind::Assistant => {
                turns.transcript_assistant_messages += 1;
                record_assistant_usage(line_number, envelope, &mut usage_by_message, &mut turns)?;
            }
            ClaudeLineKind::Other => {}
        }
    }

    if usage_by_message.is_empty() {
        return Ok(None);
    }

    let mut total = ClaudeTokenUsage::default();
    for usage in usage_by_message.values() {
        total.input_tokens += usage.input_tokens;
        total.cache_creation_input_tokens += usage.cache_creation_input_tokens;
        total.cache_read_input_tokens += usage.cache_read_input_tokens;
        total.output_tokens += usage.output_tokens;
    }

    Ok(Some(ClaudeUsageSummary {
        usage: total,
        turns,
    }))
}

fn record_assistant_usage(
    line_number: usize,
    envelope: ClaudeLine,
    usage_by_message: &mut BTreeMap<String, ClaudeUsage>,
    turns: &mut TranscriptTurnCounts,
) -> Result<()> {
    let Some(message_value) = envelope.message else {
        return Ok(());
    };
    let message: ClaudeMessage = serde_json::from_value(message_value)
        .with_context(|| format!("invalid Claude assistant message at line {line_number}"))?;
    if !message_role_is_assistant(message.role.as_ref()) {
        return Ok(());
    }
    let Some(usage) = message.usage else {
        return Ok(());
    };
    let key = match message.id {
        Some(id) => id,
        None => match envelope.uuid {
            Some(uuid) => uuid,
            None => bail!(
                "Claude assistant usage at line {line_number} has neither message.id nor top-level uuid"
            ),
        },
    };
    usage_by_message.insert(key, usage);
    turns.usage_messages += 1;
    Ok(())
}

fn message_role_is_assistant(role: Option<&ClaudeRole>) -> bool {
    matches!(role, None | Some(ClaudeRole::Assistant))
}

fn parse_lines(body: &str, description: &str) -> Result<Vec<(usize, JsonValue)>> {
    let lines = body
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .collect::<Vec<_>>();
    let mut parsed = Vec::new();
    for (position, (index, line)) in lines.iter().enumerate() {
        let line_number = index + 1;
        match serde_json::from_str::<JsonValue>(line) {
            Ok(value) => parsed.push((line_number, value)),
            Err(err) if position + 1 == lines.len() => {
                eprintln!(
                    "warning: ignored incomplete trailing {description} line {line_number}: {err}"
                );
            }
            Err(err) => bail!("malformed {description} line {line_number}: {err}"),
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cwd_slug_replaces_separators_without_slugifying() {
        assert_eq!(
            cwd_project_slug(Utf8Path::new("/Users/J/Code/Niles WT")),
            "-Users-J-Code-Niles WT"
        );
    }

    #[test]
    fn dedupes_assistant_usage_by_message_id_with_last_usage_winning() {
        let body = r#"
{"type":"user","message":{"role":"user"}}
{"type":"assistant","uuid":"line-1","message":{"id":"msg-1","role":"assistant","usage":{"input_tokens":10,"cache_creation_input_tokens":2,"cache_read_input_tokens":3,"output_tokens":4}}}
{"type":"assistant","uuid":"line-2","message":{"id":"msg-1","role":"assistant","usage":{"input_tokens":20,"cache_creation_input_tokens":5,"cache_read_input_tokens":7,"output_tokens":9}}}
{"type":"assistant","uuid":"line-3","message":{"id":"msg-2","role":"assistant","usage":{"input_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":2}}}
"#;

        let summary = parse_claude_transcript(body).unwrap().unwrap();

        assert_eq!(summary.usage.input_tokens, 21);
        assert_eq!(summary.usage.cache_creation_input_tokens, 5);
        assert_eq!(summary.usage.cache_read_input_tokens, 7);
        assert_eq!(summary.usage.output_tokens, 11);
        assert_eq!(summary.usage.total_tokens(), 44);
        assert_eq!(summary.turns.transcript_user_messages, 1);
        assert_eq!(summary.turns.transcript_assistant_messages, 3);
        assert_eq!(summary.turns.usage_messages, 3);
    }

    #[test]
    fn malformed_interior_line_fails() {
        let body = r#"
{"type":"assistant","uuid":"line-1","message":{"id":"msg-1","role":"assistant","usage":{"input_tokens":10,"cache_creation_input_tokens":2,"cache_read_input_tokens":3,"output_tokens":4}}}
{"type":"assistant"
{"type":"assistant","uuid":"line-2","message":{"id":"msg-2","role":"assistant","usage":{"input_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":2}}}
"#;

        assert!(parse_claude_transcript(body).is_err());
    }

    #[test]
    fn truncated_trailing_line_is_tolerated() {
        let body = r#"
{"type":"assistant","uuid":"line-1","message":{"id":"msg-1","role":"assistant","usage":{"input_tokens":10,"cache_creation_input_tokens":2,"cache_read_input_tokens":3,"output_tokens":4}}}
{"type":"assistant"
"#;

        let summary = parse_claude_transcript(body).unwrap().unwrap();

        assert_eq!(summary.usage.total_tokens(), 19);
    }
}
