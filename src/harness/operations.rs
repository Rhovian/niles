use super::{Capabilities, SessionId, StopReason, TurnState};
use anyhow::{Result, bail, ensure};
use serde::Deserialize;

pub(crate) struct Prompt(pub(crate) String);
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum InputDisposition {
    Unconfirmed,
    #[expect(dead_code, reason = "accepted input acknowledgment awaits #533")]
    Accepted {
        message_id: String,
    },
    #[expect(dead_code, reason = "queued input acknowledgment awaits #533")]
    Queued,
}
#[expect(dead_code, reason = "structured permission requests await #533")]
pub(crate) struct PermissionRequest {
    pub(crate) id: String,
    pub(crate) options: Vec<String>,
}
#[expect(dead_code, reason = "structured permission answers await #533")]
pub(crate) enum PermissionAnswer {
    Selected(String),
    Cancelled,
}
pub(crate) enum Update {
    #[expect(dead_code, reason = "streamed output awaits #533")]
    MessageChunk(String),
    #[expect(dead_code, reason = "streamed thoughts await #533")]
    ThoughtChunk(String),
    #[expect(dead_code, reason = "streamed tool calls await #533")]
    ToolCall { id: String, title: String },
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "streamed state transport awaits #533")
    )]
    State(TurnState),
    #[expect(dead_code, reason = "structured permission updates await #533")]
    Permission(PermissionRequest),
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Cancellation {
    #[cfg_attr(not(test), expect(dead_code, reason = "cancel transport awaits #533"))]
    Requested,
    Confirmed,
}
impl Cancellation {
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "cancel confirmation awaits #533")
    )]
    pub(crate) fn observe(&mut self, update: &Update) {
        if matches!(
            update,
            Update::State(TurnState::Idle(StopReason::Cancelled))
        ) {
            *self = Self::Confirmed;
        }
    }
}

// serde handles the wire tag; only the semantic mapping is Niles-specific.
// https://agentclientprotocol.com/protocol/prompt-turn#stop-reasons
/// v1 completion maps to idle; v1 has no accepted/queued or attention update.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum V1StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
}
impl From<V1StopReason> for TurnState {
    fn from(reason: V1StopReason) -> Self {
        Self::Idle(match reason {
            V1StopReason::EndTurn => StopReason::EndTurn,
            V1StopReason::MaxTokens => StopReason::MaxTokens,
            V1StopReason::MaxTurnRequests => StopReason::MaxTurnRequests,
            V1StopReason::Refusal => StopReason::Refusal,
            V1StopReason::Cancelled => StopReason::Cancelled,
        })
    }
}

pub(crate) trait Adapter {
    fn negotiate(&mut self) -> Result<Capabilities>;
    fn prompt(&mut self, prompt: Prompt) -> Result<InputDisposition>;
    fn cancel(&mut self) -> Result<Cancellation> {
        bail!("harness adapter declared cancel without an implementation")
    }
    fn resume(&mut self, _session: &SessionId) -> Result<()> {
        bail!("harness adapter declared resume without an implementation")
    }
    fn updates(&mut self) -> Result<Vec<Update>> {
        bail!("harness adapter has no streamed update transport")
    }
    fn answer_permission(
        &mut self,
        _request: &PermissionRequest,
        _answer: PermissionAnswer,
    ) -> Result<()> {
        bail!("harness adapter has no structured permission transport")
    }
}

/// Niles owns dispatch permission; adapters only implement the operations they advertise.
pub(crate) struct Harness<A> {
    adapter: A,
    capabilities: Capabilities,
}
impl<A: Adapter> Harness<A> {
    pub(crate) fn negotiate(mut adapter: A) -> Result<Self> {
        let capabilities = adapter.negotiate()?;
        Ok(Self {
            adapter,
            capabilities,
        })
    }
    pub(crate) fn prompt(&mut self, prompt: Prompt) -> Result<InputDisposition> {
        self.adapter.prompt(prompt)
    }
    #[expect(dead_code, reason = "streamed update dispatch awaits #533")]
    pub(crate) fn updates(&mut self) -> Result<Vec<Update>> {
        self.adapter.updates()
    }
    #[expect(dead_code, reason = "structured permission dispatch awaits #533")]
    pub(crate) fn answer_permission(
        &mut self,
        request: &PermissionRequest,
        answer: PermissionAnswer,
    ) -> Result<()> {
        self.adapter.answer_permission(request, answer)
    }
    #[cfg_attr(not(test), expect(dead_code, reason = "cancel dispatch awaits #533"))]
    pub(crate) fn cancel(&mut self) -> Result<Cancellation> {
        ensure!(self.capabilities.cancel, "harness does not support cancel");
        self.adapter.cancel()
    }
    #[cfg_attr(not(test), expect(dead_code, reason = "resume dispatch awaits #533"))]
    pub(crate) fn resume(&mut self, session: &SessionId) -> Result<()> {
        ensure!(self.capabilities.resume, "harness does not support resume");
        self.adapter.resume(session)
    }
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "attention producers await #566")
    )]
    pub(crate) fn require_attention(&self) -> Result<()> {
        ensure!(
            self.capabilities.attention,
            "harness does not support attention"
        );
        Ok(())
    }
}
