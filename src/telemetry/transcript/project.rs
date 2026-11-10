use super::{Family, TextBlock, TextRole, TextTurn};
use crate::watch;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;

// serde_json 1.0.154 decodes JSON values, not provider-specific display projections:
// https://docs.rs/serde_json/1.0.154/serde_json/value/index.html
// Claude Code 2.1.296 and pi-ai 1.2.0 dist/types.d.ts define these content blocks.
// Unknown/malformed block shapes are deliberately omitted; they never gate telemetry.
fn message_turns(
    role: TextRole,
    timestamp: Option<DateTime<Utc>>,
    content: Value,
) -> Vec<TextTurn> {
    let blocks = match content {
        Value::String(text) => vec![TextBlock::Text { text }],
        Value::Array(blocks) => blocks.into_iter().filter_map(block).collect(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => Vec::new(),
    };
    blocks
        .into_iter()
        .filter(|block| match block {
            TextBlock::Text { text } => !text.is_empty(),
            TextBlock::ToolUse { .. } => true,
        })
        .map(|content| TextTurn {
            role: if role == TextRole::User
                && matches!(&content, TextBlock::Text { text } if injected(text))
            {
                TextRole::Injected
            } else {
                role
            },
            timestamp,
            content,
        })
        .collect()
}

// Provider user-role envelopes also carry CLI-generated context, not operator input.
// Codex protocol is pinned below; Claude Code 2.1.312 uses isMeta for hook messages.
fn injected(text: &str) -> bool {
    let text = text.trim();
    watch::is_nudge(text)
        || [
            "[Request interrupted by user",
            "<command-name>",
            "<local-command-stdout>",
            "<local-command-caveat>",
            "<environment_context>",
            "<system-reminder>",
        ]
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

const MISSING_FIELD: Value = Value::Null;

fn take_optional(value: &mut Value, key: &str) -> Value {
    value.get_mut(key).map_or(MISSING_FIELD, Value::take)
}

fn block(mut value: Value) -> Option<TextBlock> {
    match value.get("type").and_then(Value::as_str) {
        None | Some("text" | "input_text" | "output_text") => Some(TextBlock::Text {
            text: value.get("text")?.as_str()?.to_owned(),
        }),
        Some("tool_use" | "toolCall") => Some(TextBlock::ToolUse {
            name: value.get("name")?.as_str()?.to_owned(),
            input: if let Some(input) = value.get_mut("input") {
                input.take()
            } else {
                take_optional(&mut value, "arguments")
            },
        }),
        Some(_) => None,
    }
}

// Model-produced function arguments need not be JSON. Preserve malformed input verbatim.
fn arguments(value: Value) -> Value {
    match value {
        Value::String(raw) => match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => Value::String(raw),
        },
        value @ (Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::Array(_)
        | Value::Object(_)) => value,
    }
}

fn function_turn(mut value: Value, timestamp: Option<DateTime<Utc>>) -> Option<TextTurn> {
    let name = value.get("name")?.as_str()?.to_owned();
    Some(TextTurn {
        role: TextRole::Assistant,
        timestamp,
        content: TextBlock::ToolUse {
            name,
            input: arguments(take_optional(&mut value, "arguments")),
        },
    })
}

// Durable response_item records avoid duplicate event_msg mirrors.
// https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/protocol/src/models.rs
pub(super) fn file_turns(family: Family, mut line: Value) -> Result<Vec<TextTurn>> {
    let timestamp = line
        .get("timestamp")
        .map(|value| serde_json::from_value::<Option<DateTime<Utc>>>(value.clone()))
        .transpose()?
        .flatten();
    let message = match (family, line.get("type").and_then(Value::as_str)) {
        (Family::Claude, Some(kind @ ("user" | "assistant"))) => {
            if line.get("isCompactSummary") == Some(&Value::Bool(true)) {
                return Ok(Vec::new());
            }
            let mut role = TextRole::parse(kind).context("Claude message role")?;
            if role == TextRole::User && line.get("isMeta") == Some(&Value::Bool(true)) {
                role = TextRole::Injected;
            }
            let content = line
                .get_mut("message")
                .map_or(MISSING_FIELD, |message| take_optional(message, "content"));
            return Ok(message_turns(role, timestamp, content));
        }
        (Family::Pi, Some("message")) => line.get_mut("message"),
        (Family::Codex, Some("response_item")) => {
            let Some(payload) = line.get_mut("payload") else {
                return Ok(Vec::new());
            };
            match payload.get("type").and_then(Value::as_str) {
                Some("message") => Some(payload),
                Some("function_call") => {
                    return Ok(function_turn(payload.take(), timestamp)
                        .into_iter()
                        .collect());
                }
                Some("custom_tool_call") => {
                    return Ok(payload
                        .get("name")
                        .and_then(Value::as_str)
                        .map(|name| TextTurn {
                            role: TextRole::Assistant,
                            timestamp,
                            content: TextBlock::ToolUse {
                                name: name.to_owned(),
                                input: payload.get("input").map_or(MISSING_FIELD, Clone::clone),
                            },
                        })
                        .into_iter()
                        .collect());
                }
                _ => return Ok(Vec::new()),
            }
        }
        _ => return Ok(Vec::new()),
    };
    let Some(message) = message else {
        return Ok(Vec::new());
    };
    let Some(role) = message
        .get("role")
        .and_then(Value::as_str)
        .and_then(TextRole::parse)
    else {
        return Ok(Vec::new());
    };
    Ok(message_turns(
        role,
        timestamp,
        take_optional(message, "content"),
    ))
}
