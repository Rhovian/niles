//! Shared adapter-boundary assertions. Native lifecycle coverage lives in the production
//! resolver/reader tests; protocol-peer lifecycle coverage is a separate follow-up, not synthetic states.
use super::{operations::*, *};
use crate::test_support::assert_err_contains;

mod native;

trait Fixture {
    type Adapter: Adapter;
    fn start(&mut self, first: &str) -> Self::Adapter;
    fn received(&self) -> String;
    fn disposition(&self) -> InputDisposition;
    fn disconnect(&mut self);
    fn restart(&mut self) -> Self::Adapter;
    fn session(&self) -> SessionId;
    fn auth_path(&self) -> &str;
}

fn run(fixture: &mut impl Fixture) {
    assert!(!fixture.auth_path().trim().is_empty());
    eprintln!("auth_path={}", fixture.auth_path());
    let first = "first input\n'$(exit 99)' \"quoted\"\ncafé 🦀";
    let adapter = fixture.start(first);
    assert_eq!(fixture.received(), first);
    let mut harness = Harness::negotiate(adapter).unwrap();
    let capabilities = harness.capabilities();
    let text = "multiline follow-up\nsecond line 🦀";
    assert_eq!(
        harness.prompt(Prompt(text.into())).unwrap(),
        fixture.disposition()
    );
    assert_eq!(fixture.received(), text);
    if !capabilities.attention {
        assert_err_contains(harness.require_attention(), "does not support attention");
        assert_err_contains(
            harness.answer_permission("permission", PermissionAnswer::Cancelled),
            "does not support attention",
        );
    }
    if !capabilities.cancel {
        assert_err_contains(harness.cancel(), "does not support cancel");
    }
    let session = fixture.session();
    if capabilities.resume {
        harness.resume(&session).unwrap();
        let missing = SessionId::new("missing-session".into()).unwrap();
        assert_err_contains(harness.resume(&missing), "resume identity");
        harness.resume(&session).unwrap();
    } else {
        assert_err_contains(harness.resume(&session), "does not support resume");
    }
    fixture.disconnect();
    assert!(harness.prompt(Prompt("disconnected".into())).is_err());
    let mut replacement = Harness::negotiate(fixture.restart()).unwrap();
    assert!(harness.prompt(Prompt("old generation".into())).is_err());
    replacement.prompt(Prompt("new generation".into())).unwrap();
    assert_eq!(fixture.received(), "new generation");
}
