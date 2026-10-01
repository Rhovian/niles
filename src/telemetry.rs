use std::{collections::HashSet, env, fs, process::Command};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Duration, Local, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::util::read_dir_utf8_paths;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "lowercase")]
pub(crate) enum SessionLink {
    Claude { session_id: String },
    Hermes { source: String },
    Codex,
}

#[derive(Debug, Serialize)]
pub(crate) struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub model: Option<String>,
    pub last_turn_at: Option<DateTime<Utc>>,
    pub estimated_cost_usd: Option<f64>,
}

impl SessionLink {
    pub(crate) fn args(&self) -> Vec<String> {
        match self {
            Self::Claude { session_id } => vec!["--session-id".into(), session_id.clone()],
            Self::Hermes { source } => vec!["--source".into(), source.clone()],
            Self::Codex => Vec::new(),
        }
    }
}

pub(crate) fn read(
    link: &SessionLink,
    workspace: &Utf8Path,
    needle: &Utf8Path,
    created_at: DateTime<Utc>,
) -> Result<Option<Usage>> {
    let home = Utf8PathBuf::from(env::var("HOME").context("HOME is missing")?);
    match link {
        SessionLink::Claude { session_id } => read_claude(&home, session_id),
        SessionLink::Hermes { source } => read_hermes(source),
        SessionLink::Codex => read_codex(&home, workspace, needle, created_at),
    }
}

fn read_claude(home: &Utf8Path, session_id: &str) -> Result<Option<Usage>> {
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

fn read_hermes(source: &str) -> Result<Option<Usage>> {
    let output = Command::new("hermes")
        .args([
            "sessions", "export", "-", "--format", "jsonl", "--source", source,
        ])
        .output()
        .context("run hermes sessions export")?;
    if !output.status.success() {
        bail!(
            "hermes sessions export failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(hermes_usage(
        &String::from_utf8(output.stdout).context("hermes export is not UTF-8")?,
    ))
}

fn read_codex(
    home: &Utf8Path,
    workspace: &Utf8Path,
    needle: &Utf8Path,
    created_at: DateTime<Utc>,
) -> Result<Option<Usage>> {
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
            let body = fs::read_to_string(&path).with_context(|| format!("read {path}"))?;
            if let Some(lines) = parse_lines::<CodexLine>(&body)
                && codex_matches(&lines, workspace, needle)
            {
                return Ok(codex_usage(&lines));
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

#[expect(
    clippy::disallowed_methods,
    reason = "unparseable session data has no usage"
)]
fn parse_lines<'a, T: Deserialize<'a>>(body: &'a str) -> Option<Vec<T>> {
    body.split_inclusive('\n')
        .filter(|line| line.ends_with('\n'))
        .map(serde_json::from_str)
        .collect::<Result<Vec<_>, _>>()
        .ok()
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeLine {
    Assistant {
        timestamp: DateTime<Utc>,
        message: ClaudeMessage,
    },
    #[serde(other)]
    Other,
}
#[derive(Deserialize)]
struct ClaudeMessage {
    id: String,
    model: String,
    usage: ClaudeCounters,
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

fn claude_usage(main: &str, subagents: &[String]) -> Option<Usage> {
    let mut seen = HashSet::new();
    let (mut input, mut output, mut cache_read, mut cache_write) = (0, 0, 0, 0);
    let (mut reasoning, mut model, mut last_turn) = (None, None, None);
    for (body, main_file) in
        std::iter::once((main, true)).chain(subagents.iter().map(|body| (body.as_str(), false)))
    {
        for line in parse_lines::<ClaudeLine>(body)? {
            let ClaudeLine::Assistant { timestamp, message } = line else {
                continue;
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
        estimated_cost_usd: None,
    })
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CodexLine {
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
    TurnContext {
        payload: CodexTurn,
    },
    #[serde(other)]
    Other,
}
#[derive(Deserialize)]
struct CodexMeta {
    cwd: Utf8PathBuf,
}
#[derive(Deserialize)]
struct CodexTurn {
    model: String,
}
#[derive(Deserialize)]
struct CodexResponse {
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
struct CodexEvent {
    #[serde(rename = "type")]
    kind: String,
    info: Option<CodexInfo>,
}
#[derive(Deserialize)]
struct CodexInfo {
    total_token_usage: Option<CodexCounters>,
}
#[derive(Deserialize)]
struct CodexCounters {
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    cache_write_input_tokens: Option<u64>,
    reasoning_output_tokens: u64,
}

fn codex_matches(lines: &[CodexLine], workspace: &Utf8Path, needle: &Utf8Path) -> bool {
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
            CodexLine::ResponseItem { .. }
            | CodexLine::EventMsg { .. }
            | CodexLine::TurnContext { .. }
            | CodexLine::Other => {}
        }
    }
    false
}

fn codex_usage(lines: &[CodexLine]) -> Option<Usage> {
    let mut last = None;
    let mut model = None;
    for line in lines {
        if let CodexLine::TurnContext { payload } = line {
            model = Some(payload.model.clone());
        }
        let CodexLine::EventMsg { timestamp, payload } = line else {
            continue;
        };
        if payload.kind != "token_count" {
            continue;
        }
        let Some(counts) = payload
            .info
            .as_ref()
            .and_then(|info| info.total_token_usage.as_ref())
        else {
            continue;
        };
        last = Some(Usage {
            input_tokens: counts.input_tokens,
            output_tokens: counts.output_tokens,
            cache_read_tokens: counts.cached_input_tokens,
            cache_write_tokens: counts.cache_write_input_tokens,
            reasoning_tokens: Some(counts.reasoning_output_tokens),
            model: None,
            last_turn_at: Some(*timestamp),
            estimated_cost_usd: None,
        });
    }
    last.map(|mut usage: Usage| {
        usage.model = model;
        usage
    })
}

#[derive(Deserialize)]
struct HermesSession {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    reasoning_tokens: u64,
    model: String,
    last_activity_at: f64,
    estimated_cost_usd: Option<f64>,
}

fn hermes_usage(body: &str) -> Option<Usage> {
    let row: HermesSession = parse_lines(body)?.into_iter().next()?;
    Some(Usage {
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        cache_read_tokens: row.cache_read_tokens,
        cache_write_tokens: Some(row.cache_write_tokens),
        reasoning_tokens: Some(row.reasoning_tokens),
        model: Some(row.model),
        last_turn_at: DateTime::from_timestamp_micros(
            (row.last_activity_at * 1_000_000.0).round() as i64
        ),
        estimated_cost_usd: row.estimated_cost_usd,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_deduplicates_and_keeps_main_model() {
        let main = "{\"type\":\"assistant\",\"timestamp\":\"2026-10-01T00:00:00Z\",\"message\":{\"id\":\"a\",\"model\":\"opus\",\"usage\":{\"input_tokens\":2,\"output_tokens\":3,\"cache_read_input_tokens\":4,\"cache_creation_input_tokens\":5}}}\n";
        let sub = main
            .replace("\"id\":\"a\"", "\"id\":\"b\"")
            .replace("opus", "haiku");
        let usage = claude_usage(&format!("{main}{main}"), &[sub]).unwrap();
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_write_tokens
            ),
            (4, 6, 8, Some(10))
        );
        assert_eq!(usage.model.as_deref(), Some("opus"));
        assert!(claude_usage("bad\n", &[]).is_none());
        assert_eq!(
            claude_usage(&format!("{main}partial"), &[])
                .unwrap()
                .input_tokens,
            2
        );
    }

    #[test]
    fn codex_first_user_and_last_total() {
        let meta = r#"{"type":"session_meta","payload":{"cwd":"/w"}}"#;
        let user = r##"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"# Niles worker brief\nreport_file: /w/report.md"}]}}"##;
        let injected = user.replace("# Niles worker brief", "# AGENTS.md instructions");
        let other = user.replace("report.md", "other.md");
        let count = |n| {
            format!(
                r#"{{"type":"event_msg","timestamp":"2026-10-01T00:00:00Z","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{n},"output_tokens":2,"cached_input_tokens":3,"reasoning_output_tokens":4}}}}}}}}"#
            )
        };
        let context = r#"{"type":"turn_context","payload":{"model":"gpt-6-sol"}}"#;
        let lines = parse_lines::<CodexLine>(&format!(
            "{meta}\n{injected}\n{user}\n{context}\n{}\n{}\n",
            count(1),
            count(9)
        ))
        .unwrap();
        assert!(codex_matches(
            &lines,
            Utf8Path::new("/w"),
            Utf8Path::new("/w/report.md")
        ));
        assert_eq!(codex_usage(&lines).unwrap().input_tokens, 9);
        assert_eq!(
            codex_usage(&lines).unwrap().model.as_deref(),
            Some("gpt-6-sol")
        );
        let lines =
            parse_lines::<CodexLine>(&format!("{meta}\n{injected}\n{other}\n{user}\n")).unwrap();
        assert!(!codex_matches(
            &lines,
            Utf8Path::new("/w"),
            Utf8Path::new("/w/report.md")
        ));
    }

    #[test]
    fn hermes_epoch_time_and_estimated_cost() {
        let row = "{\"input_tokens\":102144,\"output_tokens\":3913,\"cache_read_tokens\":277248,\"cache_write_tokens\":0,\"reasoning_tokens\":1938,\"model\":\"deepseek/deepseek-v4.1-flash\",\"last_activity_at\":1790694525.47223,\"estimated_cost_usd\":0.03}\n";
        let usage = hermes_usage(row).unwrap();
        assert_eq!(usage.input_tokens, 102144);
        assert_eq!(usage.estimated_cost_usd, Some(0.03));
        assert_eq!(
            usage.last_turn_at.unwrap().timestamp_micros(),
            1790694525472230
        );
    }
}
