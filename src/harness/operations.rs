use super::{Capabilities, SessionFailure, SessionId, StopReason, TurnState};
use agent_client_protocol::v1::schema::StopReason as WireStopReason;
use anyhow::{Result, bail, ensure};
use chrono::Utc;
use serde::Deserialize;

pub(crate) struct Prompt(pub(crate) String);
#[derive(Debug, PartialEq, Eq, serde::Serialize, Deserialize)]
pub(crate) enum InputDisposition {
    Unconfirmed,
    Accepted { message_id: String },
    Queued,
}
#[derive(serde::Serialize, Deserialize)]
pub(crate) enum PermissionAnswer {
    Selected(String),
    Cancelled,
}
impl From<WireStopReason> for TurnState {
    fn from(reason: WireStopReason) -> Self {
        Self::Idle(match reason {
            WireStopReason::EndTurn => StopReason::EndTurn,
            WireStopReason::MaxTokens => StopReason::MaxTokens,
            WireStopReason::MaxTurnRequests => StopReason::MaxTurnRequests,
            WireStopReason::Refusal => StopReason::Refusal,
            WireStopReason::Cancelled => StopReason::Cancelled,
            _ => StopReason::Error(SessionFailure {
                at: Utc::now(),
                message: format!("unknown ACP stop reason: {reason:?}"),
            }),
        })
    }
}

pub(crate) trait Adapter {
    fn negotiate(&mut self) -> Result<Capabilities>;
    fn prompt(&mut self, prompt: Prompt) -> Result<InputDisposition>;
    fn cancel(&mut self) -> Result<()> {
        bail!("harness adapter declared cancel without an implementation")
    }
    fn resume(&mut self, _session: &SessionId) -> Result<()> {
        bail!("harness adapter declared resume without an implementation")
    }
    fn answer_permission(&mut self, _request: &str, _answer: PermissionAnswer) -> Result<()> {
        bail!("harness adapter has no structured permission transport")
    }
}

/// Niles owns dispatch permission; adapters only implement the operations they advertise.
pub(crate) struct Harness<'a> {
    adapter: Box<dyn Adapter + 'a>,
    capabilities: Capabilities,
}
impl<'a> Harness<'a> {
    pub(crate) fn negotiate(mut adapter: impl Adapter + 'a) -> Result<Self> {
        let capabilities = adapter.negotiate()?;
        Ok(Self {
            adapter: Box::new(adapter),
            capabilities,
        })
    }
    pub(crate) fn prompt(&mut self, prompt: Prompt) -> Result<InputDisposition> {
        self.adapter.prompt(prompt)
    }
    pub(crate) fn answer_permission(
        &mut self,
        request: &str,
        answer: PermissionAnswer,
    ) -> Result<()> {
        self.require_attention()?;
        self.adapter.answer_permission(request, answer)
    }
    pub(crate) fn cancel(&mut self) -> Result<()> {
        ensure!(self.capabilities.cancel, "harness does not support cancel");
        self.adapter.cancel()
    }
    pub(crate) fn resume(&mut self, session: &SessionId) -> Result<()> {
        self.require_resume()?;
        self.adapter.resume(session)
    }
    pub(crate) fn require_resume(&self) -> Result<()> {
        ensure!(self.capabilities.resume, "harness does not support resume");
        Ok(())
    }
    pub(crate) fn require_attention(&self) -> Result<()> {
        ensure!(
            self.capabilities.attention,
            "harness does not support attention"
        );
        Ok(())
    }
}
