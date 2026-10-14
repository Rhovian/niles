use super::{Buckets, Turn, TurnState, Usage, parse_lines, read_transcript};
use crate::util::read_dir_utf8_paths;
use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Duration, Local, NaiveDateTime, TimeZone, Utc};
use serde::Deserialize;

pub(super) fn read_codex(
    home: &Utf8Path,
    workspace: &Utf8Path,
    needle: &Utf8Path,
    created_at: DateTime<Utc>,
) -> Result<Option<super::SessionRecord>> {
    Ok(read_codex_lines(home, workspace, needle, created_at)?
        .as_deref()
        .map(|lines| super::SessionRecord {
            turn: codex_activity(lines),
            usage: codex_usage(lines),
        }))
}

fn read_codex_lines(
    home: &Utf8Path,
    workspace: &Utf8Path,
    needle: &Utf8Path,
    created_at: DateTime<Utc>,
) -> Result<Option<Vec<CodexLine>>> {
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
        for path in read_dir_utf8_paths(&dir)? {
            let Some(stamp) = path.file_name().and_then(rollout_time) else {
                continue;
            };
            let Some(local) = Local.from_local_datetime(&stamp).single() else {
                continue;
            };
            if local + Duration::seconds(1) < start || local > deadline {
                continue;
            }
            let body = read_transcript(&path)?;
            if let Some(lines) = parse_lines::<CodexLine>(&body)
                && codex_matches(&lines, workspace, needle)
            {
                return Ok(Some(lines));
            }
        }
    }
    Ok(None)
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
    fn prompt(&self) -> u64 {
        self.input_tokens + self.cache_write_input_tokens.into_iter().sum::<u64>()
    }

    fn total(&self) -> u64 {
        self.prompt() + self.output_tokens
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

pub(super) fn codex_usage(lines: &[CodexLine]) -> Option<Usage> {
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
            .map(CodexCounters::prompt)
            .or(prompt);
        let Some(counts) = info.total_token_usage.as_ref() else {
            continue;
        };
        debug_assert!(counts.cached_input_tokens <= counts.input_tokens);
        debug_assert!(spent <= counts.total(), "Codex totals only grow");
        buckets.add(*timestamp, counts.total() - spent);
        spent = counts.total();
        last = Some((counts, *timestamp));
    }
    let (counts, timestamp) = last?;
    Some(Usage {
        input_tokens: counts.input_tokens - counts.cached_input_tokens,
        output_tokens: counts.output_tokens,
        cache_read_tokens: counts.cached_input_tokens,
        cache_write_tokens: counts.cache_write_input_tokens,
        reasoning_tokens: Some(counts.reasoning_output_tokens),
        last_turn_at: Some(timestamp),
        estimated_cost_usd: None,
        buckets,
        prompt_tokens: prompt,
        context_window: window,
    })
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
        assert!(codex_usage(&lines).is_none());
    }

    #[test]
    fn an_unparseable_rollout_is_not_this_session() {
        let home = crate::test_support::temp_test_path("codex-foreign-rollout");
        let created_at = Utc::now();
        let local = created_at.with_timezone(&Local);
        let dir = home.join(format!(".codex/sessions/{}", local.format("%Y/%m/%d")));
        std::fs::create_dir_all(&dir).unwrap();
        let name = format!("rollout-{}-x.jsonl", local.format("%Y-%m-%dT%H-%M-%S"));
        std::fs::write(dir.join(name), "bad\n").unwrap();
        let workspace = Utf8Path::new("/w");
        let found = read_codex(&home, workspace, &workspace.join("report.md"), created_at);
        assert!(found.unwrap().is_none());
        std::fs::remove_dir_all(home).unwrap();
    }
}
