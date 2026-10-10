use std::env;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

mod buckets;
mod cache;
mod claude;
mod codex;
mod hermes;
mod pi;

pub(crate) use buckets::{BUCKET, Buckets};
pub(crate) use cache::closed_buckets;
use claude::read_claude;
use codex::read_codex;
use hermes::read_hermes;
use pi::read_pi;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "lowercase")]
pub(crate) enum SessionLink {
    Claude { session_id: String },
    Hermes { source: String },
    Codex,
    Pi { session_dir: Utf8PathBuf },
}

#[derive(Debug, Serialize)]
pub(crate) struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub last_turn_at: Option<DateTime<Utc>>,
    pub state: Option<SessionState>,
    pub estimated_cost_usd: Option<f64>,
    /// `total_tokens`, spread over the buckets its turns were spent in.
    #[serde(skip)]
    pub buckets: Buckets,
    /// The latest main-transcript turn's prompt: its input, cache read and cache write tokens.
    #[serde(skip)]
    pub prompt_tokens: Option<u64>,
    /// The context window the transcript records, which only Codex does.
    #[serde(skip)]
    pub context_window: Option<u64>,
}

impl Usage {
    pub(crate) fn total_tokens(&self) -> u64 {
        self.input_tokens
            + self.cache_read_tokens
            + self.cache_write_tokens.into_iter().sum::<u64>()
            + self.output_tokens
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SessionState {
    Working,
    Waiting,
}

impl SessionLink {
    pub(crate) fn args(&self) -> Vec<String> {
        match self {
            Self::Claude { session_id } => vec!["--session-id".into(), session_id.clone()],
            Self::Hermes { source } => vec!["--source".into(), source.clone()],
            Self::Codex => Vec::new(),
            Self::Pi { session_dir } => vec!["--session-dir".into(), session_dir.to_string()],
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
        SessionLink::Hermes { source } => read_hermes(&home, source),
        SessionLink::Codex => read_codex(&home, workspace, needle, created_at),
        SessionLink::Pi { session_dir } => read_pi(session_dir),
    }
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

/// One small transcript per family, each read by that family's reader.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub(crate) const CLAUDE_MAIN: &str = concat!(
        r#"{"type":"assistant","timestamp":"2026-10-01T09:05:00Z","message":{"id":"a","stop_reason":"tool_use","usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":300,"cache_creation_input_tokens":40}}}"#,
        "\n",
        r#"{"type":"assistant","timestamp":"2026-10-01T09:05:00Z","message":{"id":"a","stop_reason":"tool_use","usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":300,"cache_creation_input_tokens":40}}}"#,
        "\n",
        r#"{"type":"assistant","timestamp":"2026-10-01T11:40:00Z","message":{"id":"b","stop_reason":"end_turn","usage":{"input_tokens":5,"output_tokens":7,"cache_read_input_tokens":600,"cache_creation_input_tokens":9}}}"#,
        "\n",
    );
    const CLAUDE_SUBAGENT: &str = concat!(
        r#"{"type":"assistant","timestamp":"2026-10-01T11:41:00Z","message":{"id":"c","usage":{"input_tokens":1000,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#,
        "\n",
    );
    const CODEX: &str = concat!(
        r#"{"type":"event_msg","timestamp":"2026-10-01T10:00:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":1},"model_context_window":1000}}}"#,
        "\n",
        r#"{"type":"event_msg","timestamp":"2026-10-01T10:00:00Z","payload":{"type":"token_count","info":null}}"#,
        "\n",
        r#"{"type":"event_msg","timestamp":"2026-10-01T13:30:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":800,"cache_write_input_tokens":50,"output_tokens":40,"reasoning_output_tokens":2},"last_token_usage":{"input_tokens":900,"cached_input_tokens":800,"cache_write_input_tokens":50,"output_tokens":30,"reasoning_output_tokens":1},"model_context_window":1000}}}"#,
        "\n",
    );
    pub(crate) const PI: &str = concat!(
        r#"{"type":"message","timestamp":"2026-10-01T08:00:00Z","message":{"role":"assistant","stopReason":"toolUse","usage":{"input":2,"output":3,"cacheRead":4,"cacheWrite":5}}}"#,
        "\n",
        r#"{"type":"message","timestamp":"2026-10-01T16:00:00Z","message":{"role":"assistant","stopReason":"stop","usage":{"input":20,"output":30,"cacheRead":40,"cacheWrite":50}}}"#,
        "\n",
    );

    pub(crate) fn claude() -> Usage {
        claude::claude_usage(CLAUDE_MAIN, &[CLAUDE_SUBAGENT.to_owned()]).unwrap()
    }

    pub(crate) fn codex() -> Usage {
        codex::codex_usage(&parse_lines(CODEX).unwrap()).unwrap()
    }

    pub(crate) fn pi() -> Usage {
        pi::pi_usage(PI).unwrap()
    }

    pub(crate) fn hermes() -> Usage {
        hermes::tests::fixture_usage()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{
        claude::claude_usage,
        codex::{CodexLine, codex_matches, codex_usage},
    };

    #[test]
    fn buckets_sum_to_the_total_for_every_family() {
        for usage in [
            fixtures::claude(),
            fixtures::codex(),
            fixtures::pi(),
            fixtures::hermes(),
        ] {
            assert_eq!(
                usage.buckets.iter().map(|(_, tokens)| tokens).sum::<u64>(),
                usage.total_tokens()
            );
            assert!(
                usage
                    .buckets
                    .iter()
                    .all(|(start, _)| start.timestamp() % 900 == 0)
            );
        }
        let claude = fixtures::claude();
        assert_eq!(
            claude
                .buckets
                .iter()
                .map(|(start, _)| start.to_rfc3339())
                .collect::<Vec<_>>(),
            ["2026-10-01T09:00:00+00:00", "2026-10-01T11:30:00+00:00"]
        );
        // The subagent's turn is spent, but the prompt is the main transcript's latest one.
        assert_eq!(claude.prompt_tokens, Some(5 + 600 + 9));
        assert_eq!(claude.context_window, None);

        let codex = fixtures::codex();
        assert_eq!(
            codex
                .buckets
                .iter()
                .map(|(_, tokens)| tokens)
                .collect::<Vec<_>>(),
            [110, 1090 - 110]
        );
        assert_eq!(codex.prompt_tokens, Some(900 + 50));
        assert_eq!(codex.context_window, Some(1000));
        assert_eq!(fixtures::pi().prompt_tokens, Some(20 + 40 + 50));

        let hermes = fixtures::hermes();
        assert_eq!(hermes.buckets.iter().count(), 1);
        assert_eq!(hermes.prompt_tokens, None);
    }

    fn state(usage: Option<Usage>) -> Option<SessionState> {
        usage.and_then(|usage| usage.state)
    }

    #[test]
    fn claude_deduplicates_usage() {
        let main = "{\"type\":\"assistant\",\"timestamp\":\"2026-10-01T00:00:00Z\",\"message\":{\"id\":\"a\",\"model\":\"opus\",\"usage\":{\"input_tokens\":2,\"output_tokens\":3,\"cache_read_input_tokens\":4,\"cache_creation_input_tokens\":5}}}\n";
        let sub = main.replace("\"id\":\"a\"", "\"id\":\"b\"");
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
        let meta =
            r#"{"type":"session_meta","timestamp":"2026-10-01T00:00:00Z","payload":{"cwd":"/w"}}"#;
        let user = r##"{"type":"response_item","timestamp":"2026-10-01T00:00:00Z","payload":{"type":"message","role":"user","content":[{"text":"# Niles worker brief\nreport_file: /w/report.md"}]}}"##;
        let injected = user.replace("# Niles worker brief", "# AGENTS.md instructions");
        let other = user.replace("report.md", "other.md");
        let count = |n| {
            format!(
                r#"{{"type":"event_msg","timestamp":"2026-10-01T00:00:00Z","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{n},"output_tokens":2,"cached_input_tokens":3,"reasoning_output_tokens":4}}}}}}}}"#
            )
        };
        let lines = parse_lines::<CodexLine>(&format!(
            "{meta}\n{injected}\n{user}\n{}\n{}\n",
            count(4),
            count(9)
        ))
        .unwrap();
        assert!(codex_matches(
            &lines,
            Utf8Path::new("/w"),
            Utf8Path::new("/w/report.md")
        ));
        assert_eq!(codex_usage(&lines).unwrap().input_tokens, 6);
        let lines =
            parse_lines::<CodexLine>(&format!("{meta}\n{injected}\n{other}\n{user}\n")).unwrap();
        assert!(!codex_matches(
            &lines,
            Utf8Path::new("/w"),
            Utf8Path::new("/w/report.md")
        ));
    }

    #[test]
    fn claude_main_lines_classify_both_states() {
        let assistant = r#"{"type":"assistant","timestamp":"2026-10-01T00:00:00Z","message":{"id":"a","model":"opus","stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
        let user = r#"{"type":"user","timestamp":"2026-10-01T00:01:00Z","message":{"role":"user","content":"next"}}"#;
        let waiting = claude_usage(&format!("{assistant}\n"), &[]).unwrap();
        assert_eq!(waiting.state, Some(SessionState::Waiting));
        let working = claude_usage(&format!("{assistant}\n{user}\n"), &[]).unwrap();
        assert_eq!(working.state, Some(SessionState::Working));
        let interrupted = r#"{"type":"user","timestamp":"2026-10-01T00:02:00Z","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#;
        let error = assistant.replace("end_turn", "stop_sequence");
        let tool_result = interrupted.replace("\"type\":\"text\"", "\"type\":\"tool_result\"");
        assert_eq!(
            state(claude_usage(&format!("{assistant}\n{interrupted}\n"), &[])),
            Some(SessionState::Waiting)
        );
        assert_eq!(
            state(claude_usage(&format!("{error}\n"), &[])),
            Some(SessionState::Waiting)
        );
        assert_eq!(
            state(claude_usage(&format!("{assistant}\n{tool_result}\n"), &[])),
            Some(SessionState::Working)
        );

        let shell = r#"{"type":"user","message":{"role":"user","content":[]},"toolUseResult":{"backgroundTaskId":"b1"}}"#;
        let notified = |body: &str| {
            format!(
                r#"{{"type":"queue-operation","operation":"enqueue","content":"<task-notification>\n<task-id>b1</task-id>\n{body}</task-notification>"}}"#
            )
        };
        let event = notified("<event>tick</event>");
        let killed = notified("<status>killed</status>");
        assert_eq!(
            state(claude_usage(
                &format!("{shell}\n{assistant}\n{event}\n"),
                &[]
            )),
            Some(SessionState::Working)
        );
        assert_eq!(
            state(claude_usage(
                &format!("{shell}\n{assistant}\n{killed}\n"),
                &[]
            )),
            Some(SessionState::Waiting)
        );
    }

    #[test]
    fn pi_message_lines_classify_both_states() {
        let assistant = r#"{"type":"message","timestamp":"2026-10-01T00:00:00Z","message":{"role":"assistant","stopReason":"stop","usage":{"input":2,"output":3,"cacheRead":4,"cacheWrite":5}}}"#;
        let next = assistant.replace("00:00:00", "00:01:00");
        let body = format!("{assistant}\n{next}\n");
        let waiting = pi::pi_usage(&body).unwrap();
        assert_eq!(waiting.state, Some(SessionState::Waiting));
        assert_eq!(
            (
                waiting.input_tokens,
                waiting.output_tokens,
                waiting.cache_read_tokens,
                waiting.cache_write_tokens
            ),
            (4, 6, 8, Some(10))
        );
        assert_eq!(
            waiting.last_turn_at,
            Some("2026-10-01T00:01:00Z".parse().unwrap())
        );
        assert!(waiting.reasoning_tokens.is_none());
        assert!(waiting.estimated_cost_usd.is_none());
        for role in ["user", "toolResult", "custom"] {
            let message = format!(
                r#"{{"type":"message","timestamp":"2026-10-01T00:02:00Z","message":{{"role":"{role}"}}}}"#
            );
            let working = pi::pi_usage(&format!("{body}{message}\n")).unwrap();
            assert_eq!(working.state, Some(SessionState::Working));
            assert_eq!(working.last_turn_at, waiting.last_turn_at);
            assert!(pi::pi_usage(&format!("{message}\n")).is_none());
        }
        assert_eq!(
            state(pi::pi_usage(&body.replace("\"stop\"", "\"toolUse\""))),
            Some(SessionState::Working)
        );
        assert_eq!(
            state(pi::pi_usage(&format!(
                "{body}{{\"type\":\"model_change\"}}\npartial"
            ))),
            Some(SessionState::Waiting)
        );
        assert!(pi::pi_usage("bad\n").is_none());
    }

    #[test]
    fn codex_events_classify_both_states() {
        let count = r#"{"type":"event_msg","timestamp":"2026-10-01T00:00:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1,"output_tokens":1,"cached_input_tokens":0,"reasoning_output_tokens":0}}}}"#;
        let started = r#"{"type":"event_msg","timestamp":"2026-10-01T00:01:00Z","payload":{"type":"task_started"}}"#;
        let complete = r#"{"type":"event_msg","timestamp":"2026-10-01T00:02:00Z","payload":{"type":"task_complete"}}"#;
        let lines = parse_lines::<CodexLine>(&format!("{count}\n{started}\n")).unwrap();
        assert_eq!(state(codex_usage(&lines)), Some(SessionState::Working));
        let lines = parse_lines::<CodexLine>(&format!("{count}\n{started}\n{complete}\n")).unwrap();
        assert_eq!(state(codex_usage(&lines)), Some(SessionState::Waiting));
    }
}
