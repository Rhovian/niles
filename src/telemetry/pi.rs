use super::{Buckets, SessionState, Usage, parse_lines};
use crate::util::read_dir_utf8_paths;
use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::fs;

pub(super) fn read_pi(session_dir: &Utf8Path) -> Result<Option<Usage>> {
    let Some(path) = read_dir_utf8_paths(session_dir)?
        .into_iter()
        .filter(|path| path.extension() == Some("jsonl"))
        .max()
    else {
        return Ok(None);
    };
    let body = fs::read_to_string(&path).with_context(|| format!("read {path}"))?;
    Ok(pi_usage(&body))
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
        usage: PiCounters,
        #[serde(rename = "stopReason")]
        stop_reason: String,
    },
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

pub(super) fn pi_usage(body: &str) -> Option<Usage> {
    let (mut input, mut output, mut cache_read, mut cache_write) = (0, 0, 0, 0);
    let (mut last_turn, mut state, mut prompt) = (None, None, None);
    let mut buckets = Buckets::default();
    for line in parse_lines::<PiLine>(body)? {
        let PiLine::Message { timestamp, message } = line else {
            continue;
        };
        state = Some(SessionState::Working);
        if let PiMessage::Assistant { usage, stop_reason } = message {
            let prompt_tokens = usage.input + usage.cache_read + usage.cache_write;
            if stop_reason != "toolUse" {
                state = Some(SessionState::Waiting);
            }
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
    last_turn?;
    Some(Usage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_write_tokens: Some(cache_write),
        reasoning_tokens: None,
        last_turn_at: last_turn,
        state,
        estimated_cost_usd: None,
        buckets,
        prompt_tokens: prompt,
        context_window: None,
    })
}
