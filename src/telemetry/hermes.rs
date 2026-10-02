use super::{SessionState, Usage, parse_lines};
use anyhow::{Context, Result, bail};
use chrono::DateTime;
use serde::Deserialize;
use std::process::Command;

pub(super) fn read_hermes(source: &str) -> Result<Option<Usage>> {
    let list = hermes(&["sessions", "list", "--source", source, "--limit", "1"])?;
    let Some(session_id) = hermes_session_id(&list) else {
        return Ok(None);
    };
    let export = hermes(&[
        "sessions",
        "export",
        "-",
        "--format",
        "jsonl",
        "--session-id",
        session_id,
    ])?;
    Ok(hermes_usage(&export))
}

fn hermes(args: &[&str]) -> Result<String> {
    let output = Command::new("hermes")
        .args(args)
        .output()
        .with_context(|| format!("run hermes {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "hermes {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8(output.stdout)
        .with_context(|| format!("hermes {} is not UTF-8", args.join(" ")))
}

pub(super) fn hermes_session_id(list: &str) -> Option<&str> {
    list.lines()
        .filter_map(|line| line.split_whitespace().last())
        .find(|id| {
            let bytes = id.as_bytes();
            bytes.len() == 22
                && bytes[..8].iter().all(u8::is_ascii_digit)
                && bytes[8] == b'_'
                && bytes[9..15].iter().all(u8::is_ascii_digit)
                && bytes[15] == b'_'
                && bytes[16..].iter().all(u8::is_ascii_hexdigit)
        })
}

#[derive(Deserialize)]
struct HermesSession {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    reasoning_tokens: u64,
    ended_at: Option<f64>,
    last_activity_at: f64,
    #[serde(default)]
    messages: Vec<HermesMessage>,
    estimated_cost_usd: Option<f64>,
}

#[derive(Deserialize)]
struct HermesMessage {
    role: String,
    tool_calls: Option<Vec<serde_json::Value>>,
}

pub(super) fn hermes_usage(body: &str) -> Option<Usage> {
    let row: HermesSession = parse_lines(body)?.into_iter().next()?;
    let state = if row.ended_at.is_some() {
        Some(SessionState::Waiting)
    } else {
        row.messages.last().map(|message| {
            if message.role == "assistant" && message.tool_calls.as_ref().is_none_or(Vec::is_empty)
            {
                SessionState::Waiting
            } else {
                SessionState::Working
            }
        })
    };
    Some(Usage {
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        cache_read_tokens: row.cache_read_tokens,
        cache_write_tokens: Some(row.cache_write_tokens),
        reasoning_tokens: Some(row.reasoning_tokens),
        last_turn_at: DateTime::from_timestamp_micros(
            (row.last_activity_at * 1_000_000.0).round() as i64
        ),
        state,
        estimated_cost_usd: row.estimated_cost_usd,
    })
}
