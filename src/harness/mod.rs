//! ACP-shaped operations; wire adapters use v1, while lifecycle meanings follow v2.
//! https://agentclientprotocol.com/protocol/v2/prompt-lifecycle
use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};

pub(crate) mod acp;
mod launch;
#[cfg(test)]
mod conformance;
pub(crate) mod native;
pub(crate) use launch::Launch;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Mode {
    #[default]
    NativeTui,
    Headless,
}

/// A conversation identity, independent of its process or display surface.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(transparent)]
pub(crate) struct SessionId(String);

impl SessionId {
    pub(crate) fn new(value: String) -> Result<Self> {
        let session = Self(value);
        ensure!(
            !session.0.trim().is_empty(),
            "empty harness session identity"
        );
        Ok(session)
    }
}

impl<'de> serde::Deserialize<'de> for SessionId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        Self::new(String::deserialize(deserializer)?).map_err(Error::custom)
    }
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SessionFailure {
    pub(crate) at: DateTime<Utc>,
    pub(crate) message: String,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    Error(SessionFailure),
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum TurnState {
    Running,
    Idle(StopReason),
    RequiresAction,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Turn {
    pub(crate) state: TurnState,
    pub(crate) started_at: Option<DateTime<Utc>>,
}

/// Declared by the adapter instance, never deduced from an agent family.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Capabilities {
    pub(crate) cancel: bool,
    pub(crate) resume: bool,
    pub(crate) attention: bool,
}

pub(crate) mod operations;
use operations::Adapter;
pub(crate) use operations::{Harness, Prompt};
