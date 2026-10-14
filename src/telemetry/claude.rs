use super::{Buckets, Turn, TurnState, Usage, parse_lines};
use crate::util::read_dir_utf8_paths;
use anyhow::Result;
use camino::Utf8Path;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashSet;

pub(super) fn read_claude(
    home: &Utf8Path,
    session_id: &str,
) -> Result<Option<super::SessionRecord>> {
    for project in read_dir_utf8_paths(&home.join(".claude/projects"))? {
        let path = project.join(format!("{session_id}.jsonl"));
        if !path.is_file() {
            continue;
        }
        let main = super::read_transcript(&path)?;
        let subagents = read_dir_utf8_paths(&project.join(session_id).join("subagents"))?
            .into_iter()
            .filter(|path| path.extension() == Some("jsonl"))
            .map(|path| super::read_transcript(&path))
            .collect::<Result<Vec<_>>>()?;
        return Ok(claude_session(&main, &subagents));
    }
    Ok(None)
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeLine {
    Assistant {
        timestamp: DateTime<Utc>,
        message: ClaudeMessage,
        #[serde(default, rename = "isApiErrorMessage")]
        api_error: bool,
    },
    User {
        timestamp: Option<DateTime<Utc>>,
        message: ClaudeUserMessage,
    },
    System {
        subtype: Option<String>,
    },
    #[serde(other)]
    Other,
}
#[derive(Deserialize)]
struct ClaudeMessage {
    id: String,
    usage: ClaudeCounters,
    stop_reason: Option<String>,
    #[serde(default)]
    content: Vec<TextBlock>,
}
#[derive(Deserialize)]
struct TextBlock {
    text: Option<String>,
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

impl ClaudeCounters {
    fn prompt(&self) -> u64 {
        self.input_tokens + self.cache_read_input_tokens + self.cache_creation_input_tokens
    }
}
#[derive(Deserialize)]
struct ClaudeDetails {
    thinking_tokens: u64,
}

impl ClaudeMessage {
    fn turn(&self, at: DateTime<Utc>, api_error: bool) -> TurnState {
        if api_error || self.stop_reason.as_deref() == Some("refusal") {
            let text = self.content.iter().find_map(|block| block.text.as_ref());
            return TurnState::Failed(super::SessionFailure {
                at,
                message: match text {
                    Some(text) => text.clone(),
                    None => "provider refusal or error (no message recorded)".into(),
                },
            });
        }
        match self.stop_reason.as_deref() {
            None | Some("tool_use") => TurnState::InProgress,
            Some(_) => TurnState::Ended,
        }
    }
}

// Claude Code 2.1.296 qDn initializes assistant IDs and usage, including error frames.
// Its SDK calls model_refusal_no_fallback a companion to the assistant refusal frame;
// only that assistant frame supplies the failure text (installed CLI schema/converter).
pub(super) fn claude_session(main: &str, subagents: &[String]) -> Option<super::SessionRecord> {
    let mut seen = HashSet::new();
    let (mut input, mut output, mut cache_read, mut cache_write) = (0, 0, 0, 0);
    let (mut reasoning, mut last_turn, mut turn, mut prompt) = (None, None, None, None);
    let mut buckets = Buckets::default();
    let mut started_at = None;
    for (body, main_file) in
        std::iter::once((main, true)).chain(subagents.iter().map(|body| (body.as_str(), false)))
    {
        for line in parse_lines::<ClaudeLine>(body)? {
            let (timestamp, message) = match line {
                ClaudeLine::Assistant {
                    timestamp,
                    message,
                    api_error,
                } => {
                    if main_file {
                        let usage = &message.usage;
                        prompt = Some(usage.prompt());
                        turn = Some(message.turn(timestamp, api_error));
                    }
                    (timestamp, message)
                }
                ClaudeLine::User { timestamp, message } => {
                    if main_file {
                        let interrupted = message.content.as_array().is_some_and(|blocks| {
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
                        });
                        if !interrupted
                            && (message.content.is_string()
                                || message.content.as_array().is_some_and(|blocks| {
                                    blocks.iter().any(|block| {
                                        matches!(block["type"].as_str(), Some("text" | "image"))
                                    })
                                }))
                        {
                            started_at = timestamp;
                        }
                        turn = Some(if interrupted {
                            TurnState::Ended
                        } else {
                            TurnState::InProgress
                        });
                    }
                    continue;
                }
                ClaudeLine::System { subtype } => {
                    if main_file
                        && matches!(
                            subtype.as_deref(),
                            Some("model_refusal_fallback" | "api_retry")
                        )
                    {
                        turn = Some(TurnState::InProgress);
                    }
                    continue;
                }
                ClaudeLine::Other => continue,
            };
            if !seen.insert(message.id) {
                continue;
            }
            let usage = message.usage;
            input += usage.input_tokens;
            output += usage.output_tokens;
            cache_read += usage.cache_read_input_tokens;
            cache_write += usage.cache_creation_input_tokens;
            buckets.add(timestamp, usage.prompt() + usage.output_tokens);
            if let Some(details) = usage.output_tokens_details {
                *reasoning.get_or_insert(0) += details.thinking_tokens;
            }
            last_turn =
                Some(last_turn.map_or(timestamp, |prior: DateTime<Utc>| prior.max(timestamp)));
        }
    }
    let usage = (!seen.is_empty()).then_some(Usage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_write_tokens: Some(cache_write),
        reasoning_tokens: reasoning,
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
mod refusal_tests {
    use super::*;
    #[test]
    fn claude_structured_error_frame_supplies_refusal_text() {
        let body = concat!(
            r#"{"type":"assistant","timestamp":"2026-10-01T00:00:00Z","isApiErrorMessage":true,"message":{"id":"refused","usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0},"stop_reason":"refusal","content":[{"type":"text","text":"Provider declined"}]}}"#,
            "\n"
        );
        let record = claude_session(body, &[]).unwrap();
        let Some(TurnState::Failed(failure)) = record.turn.map(|turn| turn.state) else {
            panic!("refusal must end the turn");
        };
        assert_eq!(failure.message, "Provider declined");
    }
}

#[cfg(test)]
mod background_tests {
    use super::*;

    #[test]
    fn halcyon_notification_before_tool_result_does_not_keep_a_turn_active() {
        let start = r#"{"type":"user","timestamp":"2026-10-13T19:00:00Z","message":{"content":[{"type":"text","text":"build"}]}}"#;
        let completed = r#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-10-13T19:00:33.090Z","content":"<task-notification><task-id>binqo6ddq</task-id><status>completed</status></task-notification>"}"#;
        let removed = r#"{"type":"queue-operation","operation":"remove","timestamp":"2026-10-13T19:00:34.208Z","reason":"absorbed_mid_turn"}"#;
        let result = r#"{"type":"user","timestamp":"2026-10-13T19:00:32.647Z","message":{"content":[{"type":"tool_result"}]},"toolUseResult":{"backgroundTaskId":"binqo6ddq"}}"#;
        let ended = r#"{"type":"assistant","timestamp":"2026-10-13T19:00:35Z","message":{"id":"a","stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
        for events in [
            format!("{completed}\n{removed}\n{result}"),
            result.to_owned(),
        ] {
            let record = claude_session(&format!("{start}\n{events}\n{ended}\n"), &[]).unwrap();
            let turn = record.turn.unwrap();
            assert!(matches!(turn.state, TurnState::Ended));
            assert_eq!(
                turn.started_at.unwrap().to_rfc3339(),
                "2026-10-13T19:00:00+00:00"
            );
        }
        let interrupted = r#"{"type":"user","timestamp":"2026-10-13T19:00:40Z","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#;
        let turn = claude_session(&format!("{start}\n{interrupted}\n"), &[])
            .unwrap()
            .turn
            .unwrap();
        assert!(matches!(turn.state, TurnState::Ended));
        assert_eq!(
            turn.started_at.unwrap().to_rfc3339(),
            "2026-10-13T19:00:00+00:00"
        );
    }
}
