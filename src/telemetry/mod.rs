use std::env;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

mod claude;
mod codex;
mod hermes;

use claude::read_claude;
use codex::read_codex;
use hermes::read_hermes;

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
    pub last_turn_at: Option<DateTime<Utc>>,
    pub state: Option<SessionState>,
    pub estimated_cost_usd: Option<f64>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use super::{
        claude::claude_usage,
        codex::{CodexLine, codex_matches, codex_usage},
    };

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
