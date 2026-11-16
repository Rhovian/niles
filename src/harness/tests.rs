use super::{operations::InputDisposition, *};
use agent_client_protocol::v1::schema::StopReason as V1StopReason;
use std::cell::Cell;

#[test]
fn v1_completion_preserves_stop_reasons() {
    for (wire, expected) in [
        ("end_turn", StopReason::EndTurn),
        ("max_tokens", StopReason::MaxTokens),
        ("max_turn_requests", StopReason::MaxTurnRequests),
        ("refusal", StopReason::Refusal),
        ("cancelled", StopReason::Cancelled),
    ] {
        let reason: V1StopReason = serde_json::from_value(serde_json::json!(wire)).unwrap();
        let state = TurnState::from(reason);
        assert_eq!(state, TurnState::Idle(expected));
    }
}

struct Permissive<'a> {
    capabilities: Capabilities,
    called: &'a Cell<u8>,
}
impl Adapter for Permissive<'_> {
    fn negotiate(&mut self) -> anyhow::Result<Capabilities> {
        Ok(self.capabilities)
    }
    fn prompt(&mut self, _: Prompt) -> anyhow::Result<InputDisposition> {
        Ok(InputDisposition::Unconfirmed)
    }
    fn cancel(&mut self) -> anyhow::Result<()> {
        self.called.set(self.called.get() | 1);
        Ok(())
    }
    fn resume(&mut self, _: &SessionId) -> anyhow::Result<()> {
        self.called.set(self.called.get() | 2);
        Ok(())
    }
}

#[test]
fn declared_capabilities_gate_operations_before_adapter_dispatch() {
    let session = SessionId::new("conversation".to_owned()).unwrap();
    for (cancel, resume, attention) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let capabilities = Capabilities {
            cancel,
            resume,
            attention,
        };
        let called = Cell::new(0);
        let mut harness = Harness::negotiate(Permissive {
            capabilities,
            called: &called,
        })
        .unwrap();
        assert_eq!(harness.cancel().is_ok(), capabilities.cancel);
        assert_eq!(harness.resume(&session).is_ok(), capabilities.resume);
        assert_eq!(harness.require_attention().is_ok(), capabilities.attention);
        assert_eq!(
            called.get(),
            u8::from(capabilities.cancel) | (u8::from(capabilities.resume) << 1)
        );
    }
}
