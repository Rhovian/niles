use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    schema::{self, ArtifactKind},
    util::write_json_pretty,
};

use super::{
    attribution::UsageAttribution,
    claude::{self, ClaudeUsageSummary},
    codex::{self, CodexResolveResult, CodexUsageSummary},
    home,
};

const CLAUDE_RESOLVED_BY: &str = "claude_session_id";
const CODEX_RESOLVED_BY: &str = "nearest_cwd_start";

#[derive(Debug, Clone)]
pub(crate) struct UsageSnapshotInput {
    pub(crate) subject: UsageSubject,
    pub(crate) agent: UsageAgent,
    pub(crate) attribution: Option<UsageAttribution>,
    pub(crate) started_at: Option<DateTime<Utc>>,
    pub(crate) finished_at: DateTime<Utc>,
    pub(crate) output_path: Utf8PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UsageSnapshot {
    pub(crate) subject: UsageSubject,
    pub(crate) agent: UsageAgent,
    pub(crate) captured_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) started_at: Option<DateTime<Utc>>,
    pub(crate) finished_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) wall_seconds: Option<i64>,
    pub(crate) attribution: UsageSnapshotAttribution,
    pub(crate) turns: UsageTurnCounts,
    pub(crate) usage: UsageSnapshotUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UsageAgent {
    pub(crate) spec: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) effort: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum UsageSubject {
    Manager {
        id: String,
    },
    Worker {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task_label: Option<String>,
    },
    RunStep {
        run_id: String,
        index: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        label: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub(crate) enum UsageSnapshotAttribution {
    ClaudeSession {
        session_id: String,
        cwd: Utf8PathBuf,
        launched_at: DateTime<Utc>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        niles_prompt_count: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transcript: Option<Utf8PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved_by: Option<String>,
    },
    CodexCwdTime {
        cwd: Utf8PathBuf,
        launched_at: DateTime<Utc>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        niles_prompt_count: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transcript: Option<Utf8PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved_by: Option<String>,
    },
    Unsupported {
        cwd: Utf8PathBuf,
        launched_at: DateTime<Utc>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        niles_prompt_count: Option<u64>,
    },
    Missing,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TranscriptTurnCounts {
    pub(crate) transcript_user_messages: u64,
    pub(crate) transcript_assistant_messages: u64,
    pub(crate) usage_messages: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UsageTurnCounts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) niles_prompt_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) transcript_user_messages: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) transcript_assistant_messages: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) usage_messages: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum UsageSnapshotUsage {
    Available {
        family: String,
        input_tokens: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cached_input_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_creation_input_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_read_input_tokens: Option<u64>,
        output_tokens: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_output_tokens: Option<u64>,
        total_tokens: u64,
    },
    Unavailable {
        reason: UsageUnavailableReason,
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UsageUnavailableReason {
    ParseError,
    Missing,
    AmbiguousCodexCandidates,
    Unsupported,
}

pub(crate) fn snapshot_usage(input: UsageSnapshotInput) -> Option<Utf8PathBuf> {
    let path = input.output_path.clone();
    let snapshot = build_snapshot(&input, true);
    match write_json_pretty(&path, &snapshot) {
        Ok(()) => Some(path),
        Err(err) => {
            eprintln!("warning: failed to write usage snapshot {path}: {err:#}");
            None
        }
    }
}

pub(crate) fn compute_usage_snapshot(input: &UsageSnapshotInput) -> UsageSnapshot {
    build_snapshot(input, false)
}

pub(crate) fn read_usage_snapshot(path: &Utf8Path) -> Result<UsageSnapshot> {
    schema::read_json(path, ArtifactKind::UsageSnapshot)
}

fn build_snapshot(input: &UsageSnapshotInput, warn_unavailable: bool) -> UsageSnapshot {
    let captured_at = Utc::now();
    let CaptureOutcome {
        attribution,
        turns,
        usage,
    } = capture_usage(input.attribution.as_ref(), warn_unavailable);

    UsageSnapshot {
        subject: input.subject.clone(),
        agent: input.agent.clone(),
        captured_at,
        started_at: input.started_at,
        finished_at: input.finished_at,
        wall_seconds: wall_seconds(input.started_at, input.finished_at),
        attribution,
        turns,
        usage,
    }
}

struct CaptureOutcome {
    attribution: UsageSnapshotAttribution,
    turns: UsageTurnCounts,
    usage: UsageSnapshotUsage,
}

fn capture_usage(attribution: Option<&UsageAttribution>, warn_unavailable: bool) -> CaptureOutcome {
    let Some(attribution) = attribution else {
        return unavailable(
            UsageSnapshotAttribution::Missing,
            None,
            UsageUnavailableReason::Unsupported,
            "no usage attribution was recorded for this artifact".to_owned(),
            warn_unavailable,
        );
    };

    match attribution {
        UsageAttribution::ClaudeSession {
            session_id,
            cwd,
            launched_at,
            niles_prompt_count,
        } => capture_claude(
            session_id,
            cwd,
            *launched_at,
            *niles_prompt_count,
            warn_unavailable,
        ),
        UsageAttribution::CodexCwdTime {
            cwd,
            launched_at,
            niles_prompt_count,
        } => capture_codex(cwd, *launched_at, *niles_prompt_count, warn_unavailable),
        UsageAttribution::Unsupported {
            cwd,
            launched_at,
            niles_prompt_count,
        } => unavailable(
            UsageSnapshotAttribution::Unsupported {
                cwd: cwd.clone(),
                launched_at: *launched_at,
                niles_prompt_count: *niles_prompt_count,
            },
            *niles_prompt_count,
            UsageUnavailableReason::Unsupported,
            "agent family does not expose a supported token ledger".to_owned(),
            warn_unavailable,
        ),
    }
}

fn capture_claude(
    session_id: &str,
    cwd: &Utf8Path,
    launched_at: DateTime<Utc>,
    niles_prompt_count: Option<u64>,
    warn_unavailable: bool,
) -> CaptureOutcome {
    let attribution_without_path = UsageSnapshotAttribution::ClaudeSession {
        session_id: session_id.to_owned(),
        cwd: cwd.to_path_buf(),
        launched_at,
        niles_prompt_count,
        transcript: None,
        resolved_by: None,
    };
    let claude_home = match home::claude_home() {
        Ok(home) => home,
        Err(err) => {
            return unavailable(
                attribution_without_path,
                niles_prompt_count,
                UsageUnavailableReason::Missing,
                format!("{err:#}"),
                warn_unavailable,
            );
        }
    };
    let transcript = claude::transcript_path(&claude_home, cwd, session_id);
    let attribution = UsageSnapshotAttribution::ClaudeSession {
        session_id: session_id.to_owned(),
        cwd: cwd.to_path_buf(),
        launched_at,
        niles_prompt_count,
        transcript: Some(transcript.clone()),
        resolved_by: Some(CLAUDE_RESOLVED_BY.to_owned()),
    };
    if !transcript.exists() {
        return unavailable(
            attribution,
            niles_prompt_count,
            UsageUnavailableReason::Missing,
            format!("Claude transcript not found at {transcript}"),
            warn_unavailable,
        );
    }
    match claude::parse_claude_transcript_file(&transcript) {
        Ok(Some(summary)) => available_claude(attribution, niles_prompt_count, summary),
        Ok(None) => unavailable(
            attribution,
            niles_prompt_count,
            UsageUnavailableReason::Missing,
            format!("Claude transcript {transcript} has no assistant usage records"),
            warn_unavailable,
        ),
        Err(err) => unavailable(
            attribution,
            niles_prompt_count,
            UsageUnavailableReason::ParseError,
            format!("{err:#}"),
            warn_unavailable,
        ),
    }
}

fn capture_codex(
    cwd: &Utf8Path,
    launched_at: DateTime<Utc>,
    niles_prompt_count: Option<u64>,
    warn_unavailable: bool,
) -> CaptureOutcome {
    let attribution_without_path = UsageSnapshotAttribution::CodexCwdTime {
        cwd: cwd.to_path_buf(),
        launched_at,
        niles_prompt_count,
        session_id: None,
        transcript: None,
        resolved_by: None,
    };
    let codex_home = match home::codex_home() {
        Ok(home) => home,
        Err(err) => {
            return unavailable(
                attribution_without_path,
                niles_prompt_count,
                UsageUnavailableReason::Missing,
                format!("{err:#}"),
                warn_unavailable,
            );
        }
    };
    let resolved = match codex::resolve_rollout(&codex_home, cwd, launched_at) {
        Ok(result) => result,
        Err(err) => {
            return unavailable(
                attribution_without_path,
                niles_prompt_count,
                UsageUnavailableReason::ParseError,
                format!("{err:#}"),
                warn_unavailable,
            );
        }
    };
    match resolved {
        CodexResolveResult::Found(transcript) => {
            let attribution = UsageSnapshotAttribution::CodexCwdTime {
                cwd: cwd.to_path_buf(),
                launched_at,
                niles_prompt_count,
                session_id: Some(transcript.session_id),
                transcript: Some(transcript.path.clone()),
                resolved_by: Some(CODEX_RESOLVED_BY.to_owned()),
            };
            match codex::parse_codex_transcript_file(&transcript.path) {
                Ok(summary) => match summary.usage {
                    Some(usage) => available_codex(attribution, niles_prompt_count, usage),
                    None => unavailable(
                        attribution,
                        niles_prompt_count,
                        UsageUnavailableReason::Missing,
                        format!(
                            "Codex rollout {} has no token_count events",
                            transcript.path
                        ),
                        warn_unavailable,
                    ),
                },
                Err(err) => unavailable(
                    attribution,
                    niles_prompt_count,
                    UsageUnavailableReason::ParseError,
                    format!("{err:#}"),
                    warn_unavailable,
                ),
            }
        }
        CodexResolveResult::Missing(detail) => unavailable(
            attribution_without_path,
            niles_prompt_count,
            UsageUnavailableReason::Missing,
            detail,
            warn_unavailable,
        ),
        CodexResolveResult::Ambiguous(detail) => unavailable(
            attribution_without_path,
            niles_prompt_count,
            UsageUnavailableReason::AmbiguousCodexCandidates,
            detail,
            warn_unavailable,
        ),
    }
}

fn available_claude(
    attribution: UsageSnapshotAttribution,
    niles_prompt_count: Option<u64>,
    summary: ClaudeUsageSummary,
) -> CaptureOutcome {
    CaptureOutcome {
        attribution,
        turns: turn_counts(niles_prompt_count, Some(summary.turns)),
        usage: UsageSnapshotUsage::Available {
            family: "claude".to_owned(),
            input_tokens: summary.usage.input_tokens,
            cached_input_tokens: None,
            cache_creation_input_tokens: Some(summary.usage.cache_creation_input_tokens),
            cache_read_input_tokens: Some(summary.usage.cache_read_input_tokens),
            output_tokens: summary.usage.output_tokens,
            reasoning_output_tokens: None,
            total_tokens: summary.usage.total_tokens(),
        },
    }
}

fn available_codex(
    attribution: UsageSnapshotAttribution,
    niles_prompt_count: Option<u64>,
    summary: CodexUsageSummary,
) -> CaptureOutcome {
    CaptureOutcome {
        attribution,
        turns: turn_counts(niles_prompt_count, Some(summary.turns)),
        usage: UsageSnapshotUsage::Available {
            family: "codex".to_owned(),
            input_tokens: summary.usage.input_tokens,
            cached_input_tokens: Some(summary.usage.cached_input_tokens),
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
            output_tokens: summary.usage.output_tokens,
            reasoning_output_tokens: Some(summary.usage.reasoning_output_tokens),
            total_tokens: summary.usage.total_tokens,
        },
    }
}

fn unavailable(
    attribution: UsageSnapshotAttribution,
    niles_prompt_count: Option<u64>,
    reason: UsageUnavailableReason,
    detail: String,
    warn_unavailable: bool,
) -> CaptureOutcome {
    if warn_unavailable {
        eprintln!("warning: usage unavailable ({reason:?}): {detail}");
    }
    CaptureOutcome {
        attribution,
        turns: turn_counts(niles_prompt_count, None),
        usage: UsageSnapshotUsage::Unavailable { reason, detail },
    }
}

fn turn_counts(
    niles_prompt_count: Option<u64>,
    transcript: Option<TranscriptTurnCounts>,
) -> UsageTurnCounts {
    match transcript {
        Some(transcript) => UsageTurnCounts {
            niles_prompt_count,
            transcript_user_messages: Some(transcript.transcript_user_messages),
            transcript_assistant_messages: Some(transcript.transcript_assistant_messages),
            usage_messages: Some(transcript.usage_messages),
        },
        None => UsageTurnCounts {
            niles_prompt_count,
            transcript_user_messages: None,
            transcript_assistant_messages: None,
            usage_messages: None,
        },
    }
}

fn wall_seconds(started_at: Option<DateTime<Utc>>, finished_at: DateTime<Utc>) -> Option<i64> {
    let started_at = started_at?;
    let seconds = (finished_at - started_at).num_seconds();
    Some(seconds.max(0))
}
