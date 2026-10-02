use super::{SessionState, Usage, parse_lines};
use crate::util::read_dir_utf8_paths;
use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::{collections::HashSet, fs};

pub(super) fn read_claude(home: &Utf8Path, session_id: &str) -> Result<Option<Usage>> {
    for project in read_dir_utf8_paths(&home.join(".claude/projects"))? {
        let path = project.join(format!("{session_id}.jsonl"));
        if !path.is_file() {
            continue;
        }
        let main = fs::read_to_string(&path).with_context(|| format!("read {path}"))?;
        let subagents = read_dir_utf8_paths(&project.join(session_id).join("subagents"))?
            .into_iter()
            .filter(|path| path.extension() == Some("jsonl"))
            .map(|path| fs::read_to_string(&path).with_context(|| format!("read {path}")))
            .collect::<Result<Vec<_>>>()?;
        return Ok(claude_usage(&main, &subagents));
    }
    Ok(None)
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeLine {
    Assistant {
        timestamp: DateTime<Utc>,
        message: ClaudeMessage,
    },
    User {
        message: ClaudeUserMessage,
    },
    #[serde(other)]
    Other,
}
#[derive(Deserialize)]
struct ClaudeMessage {
    id: String,
    model: String,
    usage: ClaudeCounters,
    stop_reason: Option<String>,
}
#[derive(Deserialize)]
struct ClaudeUserMessage {
    content: serde_json::Value,
}
#[derive(Deserialize)]
struct ClaudeCounters {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_input_tokens: u64,
    cache_creation_input_tokens: u64,
    output_tokens_details: Option<ClaudeDetails>,
}
#[derive(Deserialize)]
struct ClaudeDetails {
    thinking_tokens: u64,
}

pub(super) fn claude_usage(main: &str, subagents: &[String]) -> Option<Usage> {
    let mut seen = HashSet::new();
    let (mut input, mut output, mut cache_read, mut cache_write) = (0, 0, 0, 0);
    let (mut reasoning, mut model, mut last_turn, mut state) = (None, None, None, None);
    for (body, main_file) in
        std::iter::once((main, true)).chain(subagents.iter().map(|body| (body.as_str(), false)))
    {
        for line in parse_lines::<ClaudeLine>(body)? {
            let (timestamp, message) = match line {
                ClaudeLine::Assistant { timestamp, message } => {
                    if main_file {
                        state = Some(match message.stop_reason.as_deref() {
                            None | Some("tool_use") => SessionState::Working,
                            Some(_) => SessionState::Waiting,
                        });
                    }
                    (timestamp, message)
                }
                ClaudeLine::User { message } => {
                    if main_file {
                        state = Some(
                            if message.content.as_array().is_some_and(|blocks| {
                                blocks.iter().any(|block| {
                                    block.get("type").and_then(serde_json::Value::as_str)
                                        == Some("text")
                                        && block
                                            .get("text")
                                            .and_then(serde_json::Value::as_str)
                                            .is_some_and(|text| {
                                                text.starts_with("[Request interrupted by user")
                                            })
                                })
                            }) {
                                SessionState::Waiting
                            } else {
                                SessionState::Working
                            },
                        );
                    }
                    continue;
                }
                ClaudeLine::Other => continue,
            };
            if !seen.insert(message.id) {
                continue;
            }
            input += message.usage.input_tokens;
            output += message.usage.output_tokens;
            cache_read += message.usage.cache_read_input_tokens;
            cache_write += message.usage.cache_creation_input_tokens;
            if let Some(details) = message.usage.output_tokens_details {
                *reasoning.get_or_insert(0) += details.thinking_tokens;
            }
            if main_file {
                model = Some(message.model);
            }
            last_turn =
                Some(last_turn.map_or(timestamp, |prior: DateTime<Utc>| prior.max(timestamp)));
        }
    }
    if seen.is_empty() {
        return None;
    }
    Some(Usage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_write_tokens: Some(cache_write),
        reasoning_tokens: reasoning,
        model,
        last_turn_at: last_turn,
        state,
        estimated_cost_usd: None,
    })
}
