use super::{TokenCounts, Turn, TurnState, Usage, parse_lines, read_transcript};
use crate::util::read_dir_utf8_paths;
use anyhow::Result;
use camino::Utf8Path;
use chrono::{DateTime, Utc};
use serde::Deserialize;

pub(super) fn read_pi(session_dir: &Utf8Path) -> Result<Option<super::SessionRecord>> {
    let Some(body) = read_body(session_dir)? else {
        return Ok(None);
    };
    Ok(pi_session(&body))
}

fn read_body(session_dir: &Utf8Path) -> Result<Option<String>> {
    transcript_path(session_dir)?
        .map(|path| read_transcript(&path))
        .transpose()
}

pub(super) fn transcript_path(session_dir: &Utf8Path) -> Result<Option<camino::Utf8PathBuf>> {
    Ok(read_dir_utf8_paths(session_dir)?
        .into_iter()
        .filter(|path| path.extension() == Some("jsonl"))
        .max())
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PiLine {
    Message {
        timestamp: DateTime<Utc>,
        message: PiMessage,
    },
    #[serde(other)]
    Other,
}
#[derive(Deserialize)]
#[serde(tag = "role", rename_all = "camelCase")]
enum PiMessage {
    Assistant {
        usage: Option<TokenCounts>,
        #[serde(rename = "stopReason")]
        stop_reason: String,
        #[serde(rename = "errorMessage")]
        error_message: Option<String>,
    },
    User,
    ToolResult,
    #[serde(other)]
    Other,
}
// pi-ai 1.2.0 AssistantMessage.stopReason/errorMessage in dist/types.d.ts;
// pi-coding-agent session-manager.js persists these fields, not inferred prose.
#[expect(
    clippy::disallowed_methods,
    reason = "invalid usage rejects the transcript at its parse boundary"
)]
pub(super) fn pi_session(body: &str) -> Option<super::SessionRecord> {
    let mut usage = Usage::default();
    let lines = parse_lines::<PiLine>(body)?;
    if lines.is_empty() {
        return None;
    }
    let mut turn = None;
    let mut started_at = None;
    for line in lines {
        let PiLine::Message { timestamp, message } = line else {
            continue;
        };
        if matches!(message, PiMessage::User) {
            started_at = Some(timestamp);
        }
        turn = match &message {
            PiMessage::Other => continue,
            PiMessage::User | PiMessage::ToolResult => Some(TurnState::InProgress),
            PiMessage::Assistant {
                stop_reason,
                error_message,
                ..
            } => match stop_reason.as_str() {
                "stop" | "length" | "aborted" => Some(TurnState::Ended),
                "pending" | "toolUse" | "deferred" => Some(TurnState::InProgress),
                "error" => Some(TurnState::Failed(super::SessionFailure {
                    at: timestamp,
                    message: match error_message {
                        Some(message) => message.clone(),
                        None => "provider error (no message recorded)".into(),
                    },
                })),
                _ => None,
            },
        };
        if let PiMessage::Assistant {
            usage: Some(counts),
            ..
        } = message
        {
            usage.prompt_tokens = Some(counts.prompt().ok()?);
            usage.accumulate(timestamp, &counts).ok()?;
        }
    }
    usage.total_tokens().ok()?;
    let usage = usage.last_turn_at.map(|_| usage);
    let turn = turn.map(|state| Turn { state, started_at });
    Some(super::SessionRecord { turn, usage })
}
