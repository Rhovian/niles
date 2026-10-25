use super::{Buckets, Turn, TurnState, Usage, parse_lines, read_transcript};
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
        usage: Option<PiCounters>,
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PiCounters {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
}

// pi-ai 1.2.0 AssistantMessage.stopReason/errorMessage in dist/types.d.ts;
// pi-coding-agent session-manager.js persists these fields, not inferred prose.
pub(super) fn pi_session(body: &str) -> Option<super::SessionRecord> {
    let (mut input, mut output, mut cache_read, mut cache_write) = (0, 0, 0, 0);
    let (mut last_turn, mut prompt) = (None, None);
    let lines = parse_lines::<PiLine>(body)?;
    if lines.is_empty() {
        return None;
    }
    let mut turn = None;
    let mut started_at = None;
    let mut buckets = Buckets::default();
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
            usage: Some(usage), ..
        } = message
        {
            let prompt_tokens = usage.input + usage.cache_read + usage.cache_write;
            input += usage.input;
            output += usage.output;
            cache_read += usage.cache_read;
            cache_write += usage.cache_write;
            buckets.add(timestamp, prompt_tokens + usage.output);
            prompt = Some(prompt_tokens);
            last_turn =
                Some(last_turn.map_or(timestamp, |prior: DateTime<Utc>| prior.max(timestamp)));
        }
    }
    let usage = last_turn.map(|_| Usage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_write_tokens: Some(cache_write),
        reasoning_tokens: None,
        last_turn_at: last_turn,
        estimated_cost_usd: None,
        buckets,
        prompt_tokens: prompt,
        context_window: None,
    });
    let turn = turn.map(|state| Turn { state, started_at });
    Some(super::SessionRecord { turn, usage })
}

#[cfg(test)]
pub(super) fn pi_usage(body: &str) -> Option<Usage> {
    pi_session(body).and_then(|record| record.usage)
}
