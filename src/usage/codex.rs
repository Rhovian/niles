use std::fs;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Datelike, TimeDelta, Utc};
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::util::read_dir_utf8_paths;

use super::snapshot::TranscriptTurnCounts;

pub(crate) const CODEX_SESSION_MATCH_WINDOW_SECONDS: i64 = 120;
pub(crate) const CODEX_AMBIGUITY_EPSILON_SECONDS: i64 = 2;

const CODEX_SESSIONS_DIR: &str = "sessions";
const ROLLOUT_PREFIX: &str = "rollout-";
const JSONL_EXTENSION: &str = "jsonl";

#[derive(Debug, Clone)]
pub(crate) struct CodexResolvedTranscript {
    pub(crate) path: Utf8PathBuf,
    pub(crate) session_id: String,
}

#[derive(Debug)]
pub(crate) enum CodexResolveResult {
    Found(CodexResolvedTranscript),
    Missing(String),
    Ambiguous(String),
}

#[derive(Debug, Clone)]
pub(crate) struct CodexTranscriptSummary {
    pub(crate) session_meta: Option<CodexSessionMeta>,
    pub(crate) usage: Option<CodexUsageSummary>,
}

#[derive(Debug, Clone)]
pub(crate) struct CodexSessionMeta {
    pub(crate) session_id: String,
    pub(crate) cwd: Utf8PathBuf,
    pub(crate) timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CodexUsageSummary {
    pub(crate) usage: CodexTokenUsage,
    pub(crate) turns: TranscriptTurnCounts,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct CodexTokenUsage {
    pub(crate) input_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_output_tokens: u64,
    pub(crate) total_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct CodexLine {
    #[serde(rename = "type")]
    kind: CodexLineKind,
    payload: JsonValue,
}

#[derive(Debug)]
enum CodexLineKind {
    SessionMeta,
    EventMsg,
    Other,
}

impl<'de> Deserialize<'de> for CodexLineKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "session_meta" => Self::SessionMeta,
            "event_msg" => Self::EventMsg,
            _ => Self::Other,
        })
    }
}

#[derive(Debug, Deserialize)]
struct CodexSessionMetaPayload {
    session_id: String,
    cwd: Utf8PathBuf,
    timestamp: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct CodexEventDiscriminator {
    #[serde(rename = "type")]
    kind: CodexEventKind,
}

#[derive(Debug, Deserialize)]
struct CodexTokenEventPayload {
    #[serde(rename = "type")]
    _kind: CodexEventKind,
    info: CodexTokenInfo,
}

#[derive(Debug)]
enum CodexEventKind {
    TokenCount,
    UserMessage,
    AgentMessage,
    Other,
}

impl<'de> Deserialize<'de> for CodexEventKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "token_count" => Self::TokenCount,
            "user_message" => Self::UserMessage,
            "agent_message" => Self::AgentMessage,
            _ => Self::Other,
        })
    }
}

#[derive(Debug, Deserialize)]
struct CodexTokenInfo {
    total_token_usage: CodexTokenUsage,
}

#[derive(Debug, Clone)]
struct CodexCandidate {
    path: Utf8PathBuf,
    session_id: String,
    delta_millis: i64,
}

pub(crate) fn resolve_rollout(
    codex_home: &Utf8Path,
    cwd: &Utf8Path,
    launched_at: DateTime<Utc>,
) -> Result<CodexResolveResult> {
    let mut candidates = Vec::new();
    for dir in candidate_date_dirs(codex_home, launched_at) {
        for path in rollout_paths(&dir)? {
            let body =
                fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
            let summary = parse_codex_transcript(&body)
                .with_context(|| format!("failed to parse Codex rollout {path}"))?;
            let Some(meta) = summary.session_meta else {
                continue;
            };
            if meta.cwd != cwd {
                continue;
            }
            let delta_millis = (meta.timestamp - launched_at).num_milliseconds().abs();
            if delta_millis <= CODEX_SESSION_MATCH_WINDOW_SECONDS * 1000 {
                candidates.push(CodexCandidate {
                    path,
                    session_id: meta.session_id,
                    delta_millis,
                });
            }
        }
    }

    if candidates.is_empty() {
        return Ok(CodexResolveResult::Missing(format!(
            "no Codex rollout in cwd {cwd} within {CODEX_SESSION_MATCH_WINDOW_SECONDS}s of launch"
        )));
    }

    candidates.sort_by_key(|candidate| candidate.delta_millis);
    let nearest = match candidates.first() {
        Some(candidate) => candidate.clone(),
        None => {
            return Ok(CodexResolveResult::Missing(format!(
                "no Codex rollout in cwd {cwd} within {CODEX_SESSION_MATCH_WINDOW_SECONDS}s of launch"
            )));
        }
    };
    let ambiguous_count = candidates
        .iter()
        .filter(|candidate| {
            (candidate.delta_millis - nearest.delta_millis).abs()
                <= CODEX_AMBIGUITY_EPSILON_SECONDS * 1000
        })
        .count();
    if ambiguous_count > 1 {
        return Ok(CodexResolveResult::Ambiguous(format!(
            "{ambiguous_count} matching Codex rollouts in cwd {cwd} within {CODEX_AMBIGUITY_EPSILON_SECONDS}s of nearest launch"
        )));
    }

    Ok(CodexResolveResult::Found(CodexResolvedTranscript {
        path: nearest.path,
        session_id: nearest.session_id,
    }))
}

pub(crate) fn parse_codex_transcript_file(path: &Utf8Path) -> Result<CodexTranscriptSummary> {
    let body = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    parse_codex_transcript(&body).with_context(|| format!("failed to parse Codex rollout {path}"))
}

pub(crate) fn parse_codex_transcript(body: &str) -> Result<CodexTranscriptSummary> {
    let mut session_meta = None;
    let mut last_usage = None;
    let mut turns = TranscriptTurnCounts::default();

    for (line_number, line) in parse_lines(body, "Codex rollout")? {
        let envelope: CodexLine = serde_json::from_value(line)
            .with_context(|| format!("invalid Codex rollout line {line_number}"))?;
        match envelope.kind {
            CodexLineKind::SessionMeta => {
                let meta: CodexSessionMetaPayload = serde_json::from_value(envelope.payload)
                    .with_context(|| {
                        format!("invalid Codex session_meta payload at line {line_number}")
                    })?;
                session_meta = Some(CodexSessionMeta {
                    session_id: meta.session_id,
                    cwd: meta.cwd,
                    timestamp: meta.timestamp,
                });
            }
            CodexLineKind::EventMsg => {
                record_codex_event(line_number, envelope.payload, &mut last_usage, &mut turns)?;
            }
            CodexLineKind::Other => {}
        }
    }

    Ok(CodexTranscriptSummary {
        session_meta,
        usage: last_usage.map(|usage| CodexUsageSummary { usage, turns }),
    })
}

fn record_codex_event(
    line_number: usize,
    payload: JsonValue,
    last_usage: &mut Option<CodexTokenUsage>,
    turns: &mut TranscriptTurnCounts,
) -> Result<()> {
    let discriminator: CodexEventDiscriminator = serde_json::from_value(payload.clone())
        .with_context(|| format!("invalid Codex event payload at line {line_number}"))?;
    match discriminator.kind {
        CodexEventKind::TokenCount => {
            let event: CodexTokenEventPayload =
                serde_json::from_value(payload).with_context(|| {
                    format!("invalid Codex token_count payload at line {line_number}")
                })?;
            *last_usage = Some(event.info.total_token_usage);
            turns.usage_messages += 1;
        }
        CodexEventKind::UserMessage => turns.transcript_user_messages += 1,
        CodexEventKind::AgentMessage => turns.transcript_assistant_messages += 1,
        CodexEventKind::Other => {}
    }
    Ok(())
}

fn candidate_date_dirs(codex_home: &Utf8Path, launched_at: DateTime<Utc>) -> Vec<Utf8PathBuf> {
    let base = launched_at.date_naive();
    [-1_i64, 0, 1]
        .into_iter()
        .filter_map(|offset| base.checked_add_signed(TimeDelta::days(offset)))
        .map(|date| {
            codex_home
                .join(CODEX_SESSIONS_DIR)
                .join(format!("{:04}", date.year()))
                .join(format!("{:02}", date.month()))
                .join(format!("{:02}", date.day()))
        })
        .collect()
}

fn rollout_paths(dir: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    Ok(read_dir_utf8_paths(dir)?
        .into_iter()
        .filter(|path| path.is_file() && is_rollout(path))
        .collect())
}

fn is_rollout(path: &Utf8Path) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    name.starts_with(ROLLOUT_PREFIX) && path.extension() == Some(JSONL_EXTENSION)
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
    fn last_token_count_wins() {
        let body = r#"
{"type":"session_meta","payload":{"session_id":"s1","cwd":"/tmp/work","timestamp":"2026-07-06T01:00:00Z"}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1,"total_tokens":13}}}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":10,"total_tokens":130}}}}
"#;

        let summary = parse_codex_transcript(body).unwrap();
        let usage = summary.usage.unwrap();

        assert_eq!(usage.usage.input_tokens, 100);
        assert_eq!(usage.usage.cached_input_tokens, 20);
        assert_eq!(usage.usage.output_tokens, 30);
        assert_eq!(usage.usage.reasoning_output_tokens, 10);
        assert_eq!(usage.usage.total_tokens, 130);
        assert_eq!(usage.turns.usage_messages, 2);
    }

    #[test]
    fn malformed_interior_line_fails() {
        let body = r#"
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1,"total_tokens":13}}}}
{"type":"event_msg"
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":10,"total_tokens":130}}}}
"#;

        assert!(parse_codex_transcript(body).is_err());
    }

    #[test]
    fn truncated_trailing_line_is_tolerated() {
        let body = r#"
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1,"total_tokens":13}}}}
{"type":"event_msg"
"#;

        let summary = parse_codex_transcript(body).unwrap();

        assert_eq!(summary.usage.unwrap().usage.total_tokens, 13);
    }
}
