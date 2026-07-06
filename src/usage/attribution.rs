use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const CLAUDE_FAMILY: &str = "claude";
const CODEX_FAMILY: &str = "codex";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub(crate) enum UsageAttribution {
    ClaudeSession {
        session_id: String,
        cwd: Utf8PathBuf,
        launched_at: DateTime<Utc>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        niles_prompt_count: Option<u64>,
    },
    CodexCwdTime {
        cwd: Utf8PathBuf,
        launched_at: DateTime<Utc>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        niles_prompt_count: Option<u64>,
    },
    Unsupported {
        cwd: Utf8PathBuf,
        launched_at: DateTime<Utc>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        niles_prompt_count: Option<u64>,
    },
}

pub(crate) fn attribution_for_family(
    family: &str,
    cwd: &Utf8Path,
    launched_at: DateTime<Utc>,
    niles_prompt_count: Option<u64>,
) -> UsageAttribution {
    match family {
        CLAUDE_FAMILY => UsageAttribution::ClaudeSession {
            session_id: Uuid::new_v4().to_string(),
            cwd: cwd.to_path_buf(),
            launched_at,
            niles_prompt_count,
        },
        CODEX_FAMILY => UsageAttribution::CodexCwdTime {
            cwd: cwd.to_path_buf(),
            launched_at,
            niles_prompt_count,
        },
        _ => UsageAttribution::Unsupported {
            cwd: cwd.to_path_buf(),
            launched_at,
            niles_prompt_count,
        },
    }
}

impl UsageAttribution {
    pub(crate) fn claude_session_id(&self) -> Option<&str> {
        match self {
            UsageAttribution::ClaudeSession { session_id, .. } => Some(session_id),
            UsageAttribution::CodexCwdTime { .. } | UsageAttribution::Unsupported { .. } => None,
        }
    }
}
