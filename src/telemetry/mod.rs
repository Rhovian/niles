use crate::harness::acp;
use crate::util;
use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use zstd::stream;

mod activity;
mod buckets;
mod cache;
mod claude;
mod codex;
mod pi;
mod processes;
#[cfg_attr(
    all(not(test), not(feature = "remote")),
    expect(
        dead_code,
        reason = "remote transcript consumer is implemented in #315"
    )
)]
pub(crate) mod transcript;

pub(crate) use crate::harness::{SessionFailure, StopReason, Turn, TurnState};
pub(crate) use activity::{
    Blocker, ResolvedSession, SessionRecord, initial_deadline, resolve_session,
};
pub(crate) use buckets::{BUCKET, Buckets};
pub(crate) use cache::closed_buckets;

use claude::read_claude;
use codex::read_codex;
use pi::read_pi;
pub(crate) use processes::process_activity;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "lowercase")]
pub(crate) enum SessionLink {
    Claude { session_id: String },
    Codex,
    Acp { dir: Utf8PathBuf },
    Pi { session_dir: Utf8PathBuf },
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub last_turn_at: Option<DateTime<Utc>>,
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

/// Checked aggregation shared by provider counters and dashboard totals.
/// Iterator::sum for u64 does not return overflow errors (Rust 1.89 core/iter/traits/accum.rs).
pub(crate) fn sum(values: impl IntoIterator<Item = u64>) -> Result<u64> {
    values.into_iter().try_fold(0u64, |total, value| {
        total.checked_add(value).context("token count overflow")
    })
}

#[derive(Deserialize)]
pub(super) struct TokenCounts {
    #[serde(alias = "input")]
    input_tokens: u64,
    #[serde(alias = "output")]
    output_tokens: u64,
    #[serde(alias = "cache_read_input_tokens", alias = "cacheRead")]
    cache_read_tokens: u64,
    #[serde(alias = "cache_creation_input_tokens", alias = "cacheWrite")]
    cache_write_tokens: u64,
}

impl TokenCounts {
    fn prompt(&self) -> Result<u64> {
        sum([
            self.input_tokens,
            self.cache_read_tokens,
            self.cache_write_tokens,
        ])
    }
}

impl Usage {
    fn accumulate(&mut self, at: DateTime<Utc>, counts: &TokenCounts) -> Result<()> {
        self.input_tokens = sum([self.input_tokens, counts.input_tokens])?;
        self.output_tokens = sum([self.output_tokens, counts.output_tokens])?;
        self.cache_read_tokens = sum([self.cache_read_tokens, counts.cache_read_tokens])?;
        self.cache_write_tokens = Some(sum(self
            .cache_write_tokens
            .into_iter()
            .chain([counts.cache_write_tokens]))?);
        self.buckets
            .add(at, sum([counts.prompt()?, counts.output_tokens])?)?;
        self.last_turn_at = Some(self.last_turn_at.map_or(at, |prior| prior.max(at)));
        Ok(())
    }

    pub(crate) fn total_tokens(&self) -> Result<u64> {
        sum([
            self.input_tokens,
            self.cache_read_tokens,
            self.output_tokens,
        ]
        .into_iter()
        .chain(self.cache_write_tokens))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SessionState {
    Starting,
    Unknown,
    Working,
    Waiting,
    #[serde(rename = "needs_attention")]
    NeedsAttention,
}

impl SessionState {
    pub(crate) fn displayed(self) -> Self {
        match self {
            Self::Unknown => Self::Waiting,
            Self::Starting | Self::Working | Self::Waiting | Self::NeedsAttention => self,
        }
    }
}

impl SessionLink {
    pub(crate) fn args(&self) -> Vec<String> {
        match self {
            Self::Claude { session_id } => vec!["--session-id".into(), session_id.clone()],
            Self::Codex | Self::Acp { .. } => Vec::new(),
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
    Ok(read_session(link, workspace, needle, created_at)?.and_then(|record| record.usage))
}

pub(crate) fn read_session(
    link: &SessionLink,
    workspace: &Utf8Path,
    needle: &Utf8Path,
    created_at: DateTime<Utc>,
) -> Result<Option<SessionRecord>> {
    let home = util::home_dir()?;
    match link {
        SessionLink::Claude { session_id } => read_claude(&home, session_id),
        SessionLink::Acp { dir } => Ok(acp::turn(dir)?.map(|turn| SessionRecord {
            turn: Some(turn),
            usage: None,
        })),
        SessionLink::Codex => read_codex(&home, workspace, needle, created_at),
        SessionLink::Pi { session_dir } => read_pi(session_dir),
    }
}

fn read_transcript(path: &Utf8Path) -> Result<String> {
    (|| -> Result<String> {
        if path.as_str().ends_with(".jsonl.zst") {
            let file = File::open(path)?;
            // decode_all handles concatenated frames as well as single-frame rollouts:
            // https://docs.rs/zstd/0.13.3/zstd/stream/fn.decode_all.html
            Ok(String::from_utf8(stream::decode_all(file)?)?)
        } else {
            Ok(fs::read_to_string(path)?)
        }
    })()
    .with_context(|| format!("read {path}"))
}

/// No usage and no turn evidence: the shared resolver reports the session as Unknown, so a
/// transcript Niles cannot parse reaches the lead instead of failing every reader.
#[expect(
    clippy::disallowed_methods,
    reason = "unparseable session data has no usage and no turn evidence"
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
        claude::claude_session(CLAUDE_MAIN, &[CLAUDE_SUBAGENT.to_owned()])
            .unwrap()
            .usage
            .unwrap()
    }

    pub(crate) fn codex() -> Usage {
        let lines = parse_lines(CODEX).unwrap();
        codex::codex_usage(&lines).unwrap().unwrap()
    }

    pub(crate) fn pi() -> Usage {
        pi::pi_session(PI).unwrap().usage.unwrap()
    }
}

#[cfg(test)]
mod tests;
