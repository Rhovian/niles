use super::{Buckets, Turn, TurnState, Usage, parse_lines, read_transcript};
use crate::util::read_dir_utf8_paths;
use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Duration, Local, LocalResult, NaiveDateTime, TimeZone, Utc};
use serde::Deserialize;

pub(super) fn read_codex(
    home: &Utf8Path,
    workspace: &Utf8Path,
    needle: &Utf8Path,
    created_at: DateTime<Utc>,
) -> Result<Option<super::SessionRecord>> {
    read_codex_source(home, workspace, needle, created_at)?
        .map(|(path, lines)| {
            Ok(super::SessionRecord {
                turn: codex_activity(&lines),
                usage: codex_usage(&lines)
                    .with_context(|| format!("invalid Codex transcript {path}"))?,
            })
        })
        .transpose()
}

pub(super) fn read_codex_source(
    home: &Utf8Path,
    workspace: &Utf8Path,
    needle: &Utf8Path,
    created_at: DateTime<Utc>,
) -> Result<Option<(Utf8PathBuf, Vec<CodexLine>)>> {
    let start = created_at.with_timezone(&Local);
    let deadline = start + Duration::minutes(2);
    for day in [start.date_naive(), deadline.date_naive()]
        .into_iter()
        .enumerate()
    {
        let (index, day) = day;
        if index == 1 && day == start.date_naive() {
            break;
        }
        let dir = home.join(format!(".codex/sessions/{}", day.format("%Y/%m/%d")));
        let paths = read_dir_utf8_paths(&dir)?;
        for path in &paths {
            // Directory entries are sorted paths, not logical rollouts. Prefer the live
            // plain representation, as Codex does when materializing for append:
            // https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/rollout/src/compression.rs
            if let Some(plain) = path.as_str().strip_suffix(".jsonl.zst") {
                if paths
                    .binary_search(&Utf8PathBuf::from(format!("{plain}.jsonl")))
                    .is_ok()
                {
                    continue;
                }
            } else if path.extension() != Some("jsonl") {
                continue;
            }
            let Some(stamp) = path.file_name().and_then(rollout_time) else {
                continue;
            };
            if !rollout_in_window(Local.from_local_datetime(&stamp), start) {
                continue;
            }
            let body = read_transcript(path)?;
            if let Some(lines) = parse_lines::<CodexLine>(&body)
                && codex_matches(&lines, workspace, needle)
            {
                return Ok(Some((path.clone(), lines)));
            }
        }
    }
    Ok(None)
}

fn rollout_in_window(candidates: LocalResult<DateTime<Local>>, start: DateTime<Local>) -> bool {
    [candidates.earliest(), candidates.latest()]
        .into_iter()
        .flatten()
        .any(|local| local + Duration::seconds(1) >= start && local <= start + Duration::minutes(2))
}

#[expect(clippy::disallowed_methods, reason = "non-rollout names are skipped")]
fn rollout_time(name: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(
        name.strip_prefix("rollout-")?.get(..19)?,
        "%Y-%m-%dT%H-%M-%S",
    )
    .ok()
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum CodexLine {
    SessionMeta {
        payload: CodexMeta,
    },
    ResponseItem {
        payload: CodexResponse,
    },
    EventMsg {
        timestamp: DateTime<Utc>,
        payload: CodexEvent,
    },
    #[serde(other)]
    Other,
}
#[derive(Deserialize)]
pub(super) struct CodexMeta {
    cwd: Utf8PathBuf,
}
#[derive(Deserialize)]
pub(super) struct CodexResponse {
    #[serde(rename = "type")]
    kind: String,
    role: Option<String>,
    content: Option<Vec<CodexContent>>,
}
#[derive(Deserialize)]
struct CodexContent {
    text: Option<String>,
}
#[derive(Deserialize)]
pub(super) struct CodexEvent {
    #[serde(rename = "type")]
    kind: String,
    error: Option<CodexError>,
    info: Option<CodexInfo>,
}
#[derive(Deserialize)]
struct CodexInfo {
    total_token_usage: Option<CodexCounters>,
    last_token_usage: Option<CodexCounters>,
    model_context_window: Option<u64>,
}
#[derive(Deserialize)]
struct CodexCounters {
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    cache_write_input_tokens: Option<u64>,
    reasoning_output_tokens: u64,
}

impl CodexCounters {
    /// Codex's input count already includes its cached part.
    fn checked(&self) -> Result<Usage> {
        let input_tokens = self
            .input_tokens
            .checked_sub(self.cached_input_tokens)
            .context("Codex cached input exceeds input tokens")?;
        let prompt = self
            .input_tokens
            .checked_add(self.cache_write_input_tokens.into_iter().sum())
            .context("Codex prompt token count overflows")?;
        prompt
            .checked_add(self.output_tokens)
            .context("Codex total token count overflows")?;
        Ok(Usage {
            input_tokens,
            output_tokens: self.output_tokens,
            cache_read_tokens: self.cached_input_tokens,
            cache_write_tokens: self.cache_write_input_tokens,
            reasoning_tokens: Some(self.reasoning_output_tokens),
            last_turn_at: None,
            estimated_cost_usd: None,
            buckets: Buckets::default(),
            prompt_tokens: Some(prompt),
            context_window: None,
        })
    }
}

pub(super) fn codex_matches(lines: &[CodexLine], workspace: &Utf8Path, needle: &Utf8Path) -> bool {
    let mut cwd_matches = false;
    for line in lines {
        match line {
            CodexLine::SessionMeta { payload } => cwd_matches = payload.cwd == workspace,
            CodexLine::ResponseItem { payload }
                if payload.kind == "message" && payload.role.as_deref() == Some("user") =>
            {
                let Some(items) = payload.content.as_ref() else {
                    continue;
                };
                for item in items {
                    if let Some(text) = item.text.as_deref()
                        && text.starts_with("# Niles ")
                    {
                        return cwd_matches && text.contains(needle.as_str());
                    }
                }
            }
            CodexLine::ResponseItem { .. } | CodexLine::EventMsg { .. } | CodexLine::Other => {}
        }
    }
    false
}

pub(super) fn codex_usage(lines: &[CodexLine]) -> Result<Option<Usage>> {
    let (mut last, mut prompt, mut window, mut spent) = (None, None, None, 0);
    let mut buckets = Buckets::default();
    for line in lines {
        let CodexLine::EventMsg { timestamp, payload } = line else {
            continue;
        };
        if payload.kind != "token_count" {
            continue;
        }
        let Some(info) = payload.info.as_ref() else {
            continue;
        };
        window = info.model_context_window.or(window);
        prompt = info
            .last_token_usage
            .as_ref()
            .map(CodexCounters::checked)
            .transpose()?
            .and_then(|usage| usage.prompt_tokens)
            .or(prompt);
        let Some(counts) = info.total_token_usage.as_ref() else {
            continue;
        };
        let mut usage = counts
            .checked()
            .with_context(|| format!("invalid Codex token record at {timestamp}"))?;
        let total = usage.total_tokens();
        let delta = total
            .checked_sub(spent)
            .context("Codex token totals decreased")?;
        buckets.add(*timestamp, delta);
        spent = total;
        usage.last_turn_at = Some(*timestamp);
        last = Some(usage);
    }
    Ok(last.map(|usage| Usage {
        buckets,
        prompt_tokens: prompt,
        context_window: window,
        ..usage
    }))
}

#[derive(Deserialize)]
struct CodexError {
    message: String,
}

// Terminal errors are durable TurnCompleteEvent.error records; standalone ErrorEvent
// is transient and is not a turn-end signal (rollout/src/policy.rs in the same pinned source).
// https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/protocol/src/protocol.rs
pub(super) fn codex_activity(lines: &[CodexLine]) -> Option<Turn> {
    let mut turn = None;
    let mut started_at = None;
    for line in lines {
        let CodexLine::EventMsg { timestamp, payload } = line else {
            continue;
        };
        match payload.kind.as_str() {
            "task_started" => {
                started_at = Some(*timestamp);
                turn = Some(TurnState::InProgress);
            }
            "task_complete" => {
                turn = Some(match &payload.error {
                    Some(error) => TurnState::Failed(super::SessionFailure {
                        at: *timestamp,
                        message: error.message.clone(),
                    }),
                    None => TurnState::Ended,
                })
            }
            "turn_aborted" => turn = Some(TurnState::Ended),
            _ => {}
        }
    }
    turn.map(|state| Turn { state, started_at })
}

#[cfg(test)]
mod refusal_tests {
    use super::*;
    #[test]
    fn codex_terminal_error_is_failure_without_usage() {
        let body = concat!(
            r#"{"type":"event_msg","timestamp":"2026-10-01T00:00:00Z","payload":{"type":"task_complete","error":{"message":"Provider stopped","codex_error_info":"cyber_policy"}}}"#,
            "\n"
        );
        let lines = parse_lines::<CodexLine>(body).unwrap();
        let Some(TurnState::Failed(failure)) = codex_activity(&lines).map(|turn| turn.state) else {
            panic!("terminal error must end the turn");
        };
        assert_eq!(failure.message, "Provider stopped");
        assert!(codex_usage(&lines).unwrap().is_none());
    }

    #[test]
    fn rollout_window_checks_both_fold_instants_and_bounds() {
        let first = Utc::now().with_timezone(&Local);
        let second = first + Duration::hours(1);
        for (seconds, expected) in [(0, true), (3600, true), (-121, false), (3602, false)] {
            assert_eq!(
                rollout_in_window(
                    LocalResult::Ambiguous(first, second),
                    first + Duration::seconds(seconds)
                ),
                expected
            );
        }
    }
}
