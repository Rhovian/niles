use super::{Family, TextBlock, TextRole, TextTurn};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;

// serde_json 1.0.151 decodes JSON values, not provider-specific display projections:
// https://docs.rs/serde_json/1.0.151/serde_json/value/index.html
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
            role,
            timestamp,
            content,
        })
        .collect()
}

fn block(mut value: Value) -> Option<TextBlock> {
    match value["type"].as_str() {
        None | Some("text" | "input_text" | "output_text") => Some(TextBlock::Text {
            text: value["text"].as_str()?.to_owned(),
        }),
        Some("tool_use" | "toolCall") => Some(TextBlock::ToolUse {
            name: value["name"].as_str()?.to_owned(),
            input: if value.get("input").is_some() {
                value["input"].take()
            } else {
                match value.get_mut("arguments") {
                    Some(input) => input.take(),
                    None => Value::Null,
                }
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
    let name = value["name"].as_str()?.to_owned();
    Some(TextTurn {
        role: TextRole::Assistant,
        timestamp,
        content: TextBlock::ToolUse {
            name,
            input: arguments(value["arguments"].take()),
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
    let message = match (family, line["type"].as_str()) {
        (Family::Claude, Some("user" | "assistant")) => {
            if line["isCompactSummary"] == true {
                return Ok(Vec::new());
            }
            let role = TextRole::parse(line["type"].as_str().context("Claude message type")?)
                .context("Claude message role")?;
            let content = match line["message"].get_mut("content") {
                Some(content) => content.take(),
                None => Value::Null,
            };
            return Ok(message_turns(role, timestamp, content));
        }
        (Family::Pi, Some("message")) => &mut line["message"],
        (Family::Codex, Some("response_item")) => {
            let payload = &mut line["payload"];
            match payload["type"].as_str() {
                Some("message") => payload,
                Some("function_call") => {
                    return Ok(function_turn(payload.take(), timestamp)
                        .into_iter()
                        .collect());
                }
                Some("custom_tool_call") => {
                    return Ok(payload["name"]
                        .as_str()
                        .map(|name| TextTurn {
                            role: TextRole::Assistant,
                            timestamp,
                            content: TextBlock::ToolUse {
                                name: name.to_owned(),
                                input: payload["input"].clone(),
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
    let Some(role) = message["role"].as_str().and_then(TextRole::parse) else {
        return Ok(Vec::new());
    };
    Ok(message_turns(role, timestamp, message["content"].take()))
}
