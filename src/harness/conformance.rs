//! Shared acceptance runner. A fixture drives the outside CLI/protocol, never the adapter's
//! answers. ACP fixtures can implement Fixture beside the native fixture below.
use super::{operations::*, *};
use crate::test_support::{assert_err_contains, at};

mod native;

pub(super) enum Signal {
    Running,
    Failure(&'static str),
    Attention,
    Cancelled,
}

pub(super) trait Fixture {
    type Adapter: Adapter;
    fn start(&mut self, first: &str) -> Self::Adapter;
    fn received(&self) -> String;
    fn disposition(&self, busy: bool) -> InputDisposition;
    fn signal(&mut self, signal: Signal);
    fn observe(&mut self, seconds: i64) -> TurnState;
    fn disconnect(&mut self);
    fn restart(&mut self) -> Self::Adapter;
    fn stale_event(&mut self);
    fn session(&self) -> SessionId;
    fn permission(&self) -> PermissionRequest;
    fn auth_path(&self) -> &str;
}

pub(super) fn run(fixture: &mut impl Fixture) {
    assert!(!fixture.auth_path().trim().is_empty());
    eprintln!("auth_path={}", fixture.auth_path());
    let first = "first input\n'$(exit 99)' \"quoted\"\ncafé 🦀";
    let adapter = fixture.start(first);
    assert_eq!(fixture.received(), first);
    let mut harness = Harness::negotiate(adapter).unwrap();
    let capabilities = harness.capabilities();
    for busy in [false, true] {
        if busy {
            fixture.signal(Signal::Running);
        }
        let text = "multiline follow-up\nsecond line 🦀";
        assert_eq!(
            harness.prompt(Prompt(text.into())).unwrap(),
            fixture.disposition(busy)
        );
        assert_eq!(fixture.received(), text);
    }
    // Advance the observation clock, not wall time: quiet is never turn completion.
    assert_eq!(fixture.observe(1000), TurnState::Running);
    assert_eq!(fixture.observe(100000), TurnState::Running);
    for message in ["provider failed", "quota exhausted"] {
        fixture.signal(Signal::Failure(message));
        assert_eq!(
            fixture.observe(100000),
            TurnState::Idle(StopReason::Error(SessionFailure {
                at: at(1000),
                message: message.into(),
            }))
        );
    }
    if capabilities.attention {
        harness.require_attention().unwrap();
        fixture.signal(Signal::Attention);
        for now in [1000, 100000] {
            assert_eq!(fixture.observe(now), TurnState::RequiresAction);
        }
        let request = fixture.permission();
        harness
            .answer_permission(&request, PermissionAnswer::Selected("allow".into()))
            .unwrap();
        assert_eq!(fixture.observe(100000), TurnState::Running);
    } else {
        assert_err_contains(harness.require_attention(), "does not support attention");
        // Also exercise the operation a caller might attempt after a failed negotiation.
        assert_err_contains(
            harness.answer_permission(&fixture.permission(), PermissionAnswer::Cancelled),
            "does not support attention",
        );
    }
    fixture.signal(Signal::Running);
    if capabilities.cancel {
        let mut cancellation = harness.cancel().unwrap();
        assert_eq!(cancellation, Cancellation::Requested);
        cancellation.observe(&Update::State(fixture.observe(100000)));
        assert_eq!(cancellation, Cancellation::Requested);
        fixture.signal(Signal::Cancelled);
        cancellation.observe(&Update::State(fixture.observe(100000)));
        assert_eq!(cancellation, Cancellation::Confirmed);
    } else {
        assert_err_contains(harness.cancel(), "does not support cancel");
    }
    assert_err_contains(
        SessionId::new(String::new()),
        "empty harness session identity",
    );
    let session = fixture.session();
    if capabilities.resume {
        harness.resume(&session).unwrap();
        assert_err_contains(
            harness.resume(&SessionId::new("missing-session".into()).unwrap()),
            "missing",
        );
        assert_eq!(fixture.session(), session);
    } else {
        assert_err_contains(harness.resume(&session), "does not support resume");
    }
    fixture.disconnect();
    assert!(harness.prompt(Prompt("disconnected".into())).is_err());
    let mut replacement = Harness::negotiate(fixture.restart()).unwrap();
    fixture.signal(Signal::Running);
    fixture.stale_event();
    assert_eq!(fixture.observe(100000), TurnState::Running);
    // A previous process generation must never receive input intended for its replacement.
    assert!(harness.prompt(Prompt("old generation".into())).is_err());
    replacement.prompt(Prompt("new generation".into())).unwrap();
    assert_eq!(fixture.received(), "new generation");
}
