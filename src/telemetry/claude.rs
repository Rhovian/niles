use super::{Buckets, SessionState, Usage, parse_lines};
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
        #[serde(rename = "toolUseResult")]
        tool_use_result: Option<serde_json::Value>,
    },
    /// Queued input, including the `<task-notification>` for each background task event.
    #[serde(rename = "queue-operation")]
    QueueOperation { content: Option<serde_json::Value> },
    #[serde(other)]
    Other,
}
#[derive(Deserialize)]
struct ClaudeMessage {
    id: String,
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

impl ClaudeCounters {
    fn prompt(&self) -> u64 {
        self.input_tokens + self.cache_read_input_tokens + self.cache_creation_input_tokens
    }
}
#[derive(Deserialize)]
struct ClaudeDetails {
    thinking_tokens: u64,
}

pub(super) fn claude_usage(main: &str, subagents: &[String]) -> Option<Usage> {
    let mut seen = HashSet::new();
    let (mut input, mut output, mut cache_read, mut cache_write) = (0, 0, 0, 0);
    let (mut reasoning, mut last_turn, mut state, mut prompt) = (None, None, None, None);
    let mut buckets = Buckets::default();
    let mut background_shells = HashSet::new();
    for (body, main_file) in
        std::iter::once((main, true)).chain(subagents.iter().map(|body| (body.as_str(), false)))
    {
        for line in parse_lines::<ClaudeLine>(body)? {
            let (timestamp, message) = match line {
                ClaudeLine::Assistant { timestamp, message } => {
                    if main_file {
                        let usage = &message.usage;
                        prompt = Some(usage.prompt());
                        state = Some(match message.stop_reason.as_deref() {
                            None | Some("tool_use") => SessionState::Working,
                            Some(_) => SessionState::Waiting,
                        });
                    }
                    (timestamp, message)
                }
                ClaudeLine::User {
                    message,
                    tool_use_result,
                } => {
                    if main_file {
                        if let Some(id) = tool_use_result
                            .as_ref()
                            .and_then(|result| result.get("backgroundTaskId"))
                            .and_then(serde_json::Value::as_str)
                        {
                            background_shells.insert(id.to_owned());
                        }
                        if let Some(id) = tool_use_result.as_ref().and_then(stopped_task) {
                            background_shells.remove(id);
                        }
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
                ClaudeLine::QueueOperation { content } => {
                    if main_file
                        && let Some(id) = content
                            .as_ref()
                            .and_then(serde_json::Value::as_str)
                            .and_then(ended_task)
                    {
                        background_shells.remove(id);
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
    if seen.is_empty() {
        return None;
    }
    // A turn that ended with a shell still running wakes itself when the shell finishes.
    if state == Some(SessionState::Waiting) && !background_shells.is_empty() {
        state = Some(SessionState::Working);
    }
    Some(Usage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_write_tokens: Some(cache_write),
        reasoning_tokens: reasoning,
        last_turn_at: last_turn,
        state,
        estimated_cost_usd: None,
        buckets,
        prompt_tokens: prompt,
        context_window: None,
    })
}

/// The task a notification reports as ended. Monitor events notify without a `<status>` while
/// the task keeps running.
fn ended_task(notification: &str) -> Option<&str> {
    if !notification.contains("<status>") {
        return None;
    }
    let (_, rest) = notification.split_once("<task-id>")?;
    Some(rest.split_once("</task-id>")?.0)
}

fn stopped_task(result: &serde_json::Value) -> Option<&str> {
    let message = result.get("message")?.as_str()?;
    if !message.starts_with("Successfully stopped task: ") {
        return None;
    }
    result.get("task_id")?.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    const END_TURN: &str = r#"{"type":"assistant","timestamp":"2026-10-01T00:00:00Z","message":{"id":"a","stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;

    fn user_result(result: serde_json::Value) -> String {
        serde_json::json!({
            "type": "user",
            "message": {"content": []},
            "toolUseResult": result,
        })
        .to_string()
    }

    fn shell_and_stop() -> (String, String) {
        let shell = user_result(serde_json::json!({"backgroundTaskId": "b1"}));
        let stopped = user_result(serde_json::json!({
            "message": "Successfully stopped task: b1 (sleep 60)",
            "task_id": "b1",
            "task_type": "local_bash",
            "command": "sleep 60",
        }));
        (shell, stopped)
    }

    #[test]
    fn task_stop_releases_background_shell_without_notification() {
        let (shell, stopped) = shell_and_stop();
        let transcript = format!("{shell}\n{stopped}\n{END_TURN}\n");
        assert_eq!(
            claude_usage(&transcript, &[]).unwrap().state,
            Some(SessionState::Waiting)
        );
    }

    #[test]
    fn task_stop_preserves_other_running_shells_and_active_turns() {
        let (shell, stopped) = shell_and_stop();
        for result in [
            serde_json::json!({"task_id": "b1", "message": "Task is still running"}),
            serde_json::json!({"task_id": "b2", "message": "Successfully stopped task: b2"}),
            serde_json::json!({"message": "Successfully stopped task: b1"}),
        ] {
            let result = user_result(result);
            assert_eq!(
                claude_usage(&format!("{shell}\n{result}\n{END_TURN}\n"), &[])
                    .unwrap()
                    .state,
                Some(SessionState::Working)
            );
        }
        let other_shell = user_result(serde_json::json!({"backgroundTaskId": "b2"}));
        assert_eq!(
            claude_usage(
                &format!("{shell}\n{other_shell}\n{stopped}\n{END_TURN}\n"),
                &[]
            )
            .unwrap()
            .state,
            Some(SessionState::Working)
        );
        assert_eq!(
            claude_usage(&format!("{END_TURN}\n{shell}\n{stopped}\n"), &[])
                .unwrap()
                .state,
            Some(SessionState::Working)
        );
        assert_eq!(
            claude_usage(&format!("{shell}\n{END_TURN}\n"), &[format!("{stopped}\n")])
                .unwrap()
                .state,
            Some(SessionState::Working)
        );
    }
}
