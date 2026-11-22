//! Shared adapter-boundary assertions using real tmux panes and ACP peers.
use super::{operations::*, *};
use crate::agents::{AgentSpec, ModelCatalogue};
use crate::test_support::assert_err_contains;
use anyhow::{Context, ensure};
use camino::Utf8Path;
use std::env;
use std::io::ErrorKind;
use std::process::Command;
use std::time::{Duration, Instant};
use std::{fs, thread};

mod acp;
mod native;

trait Fixture {
    type Generation;
    fn start(&mut self, first: &str) -> Self::Generation;
    fn restart(&mut self, previous: &Self::Generation, prompt: &str) -> Self::Generation;
    fn connect(generation: &Self::Generation) -> Result<impl Adapter + '_>;
    fn received(&self) -> String;
    fn disposition(&self) -> InputDisposition;
    fn capabilities(&self) -> Capabilities;
    fn lifecycle_peer(&self) -> Option<&dyn LifecyclePeer> {
        None
    }
    fn disconnect(&mut self);
    fn finish(&mut self) {
        self.disconnect();
    }
    fn session(&self) -> SessionId;
    fn auth_path(&self) -> &str;
}

enum LifecycleCoverage {
    Full,
    SharedAcpRunner,
}

trait LifecyclePeer {
    fn set_history_missing(&self, missing: bool);
    fn receipt(&self) -> String;
    fn wait_turn(&self, state: TurnState);
    fn transcript(&self) -> String;
}

// ACP control connections serve one operation apiece, as in worker/pane.rs.
fn harness<F: Fixture>(generation: &F::Generation) -> Result<Harness<'_>> {
    Harness::negotiate(F::connect(generation)?)
}

fn run<F: Fixture>(fixture: &mut F, coverage: LifecycleCoverage) {
    assert!(!fixture.auth_path().trim().is_empty());
    eprintln!("auth_path={}", fixture.auth_path());
    let first = "first input\n'$(exit 99)' \"quoted\"\ncafé 🦀";
    let generation = fixture.start(first);
    assert_eq!(fixture.received(), first);
    let client = || harness::<F>(&generation).unwrap();
    let mut first_client = client();
    let capabilities = first_client.capabilities();
    assert_eq!(capabilities, fixture.capabilities());
    if !capabilities.attention {
        assert_err_contains(client().require_attention(), "does not support attention");
        assert_err_contains(
            client().answer_permission("permission", PermissionAnswer::Cancelled),
            "does not support attention",
        );
    }
    if !capabilities.cancel {
        assert_err_contains(client().cancel(), "does not support cancel");
    }
    let session = fixture.session();
    if capabilities.resume {
        first_client.resume(&session).unwrap();
        let missing = SessionId::new("missing-session".into()).unwrap();
        assert_err_contains(client().resume(&missing), "resume identity");
        client().resume(&session).unwrap();
    } else {
        assert_err_contains(first_client.resume(&session), "does not support resume");
    }

    let text = "multiline follow-up\nsecond line 🦀";
    assert_eq!(
        client().prompt(Prompt(text.into())).unwrap(),
        fixture.disposition()
    );
    assert_eq!(fixture.received(), text);

    // Pin a live transport before disconnecting. A new connection to a rebound
    // ACP address would reach the replacement and could not probe this generation.
    let mut retired = client();
    fixture.disconnect();
    assert!(
        harness::<F>(&generation)
            .and_then(|mut harness| harness.prompt(Prompt("disconnected".into())))
            .is_err()
    );
    let replacement = fixture.restart(&generation, first);
    assert!(retired.prompt(Prompt("old generation".into())).is_err());
    let mut replacement_client = harness::<F>(&replacement).unwrap();
    assert_eq!(replacement_client.capabilities(), capabilities);
    if capabilities.resume {
        assert_eq!(fixture.session(), session);
    }
    replacement_client
        .prompt(Prompt("new generation".into()))
        .unwrap();
    assert_eq!(fixture.received(), "new generation");

    if matches!(coverage, LifecycleCoverage::Full)
        && (capabilities.attention || capabilities.cancel || capabilities.resume)
    {
        check_lifecycle(fixture, &replacement, capabilities);
    }
    fixture.finish();
}

fn check_lifecycle<F: Fixture>(
    fixture: &F,
    generation: &F::Generation,
    capabilities: Capabilities,
) {
    const APPROVAL: (&str, &str, &str) = ("approval", "100", "yes");
    const HELD: &str = "hold for cancel";
    const CANCEL_RECEIPT: &str = "cancel requested";
    const HISTORY_PROBE: &str = "history probe";
    const OPENING_REPLY: &str = "peer opening reply";
    const HISTORY_REPLY: &str = "loaded opening history";
    const MISSING_HISTORY: &str = "missing peer history";

    let peer = fixture
        .lifecycle_peer()
        .expect("supported lifecycle needs a peer");
    let client = || harness::<F>(generation).unwrap();
    if capabilities.resume {
        client().prompt(Prompt(HISTORY_PROBE.into())).unwrap();
        assert_eq!(fixture.received(), HISTORY_PROBE);
        let transcript = peer.transcript();
        assert_eq!(transcript.matches(OPENING_REPLY).count(), 1);
        assert!(transcript.contains(HISTORY_REPLY));

        peer.set_history_missing(true);
        assert_err_contains(client().resume(&fixture.session()), MISSING_HISTORY);
        peer.set_history_missing(false);
    }
    if capabilities.attention {
        let (prompt, request, option) = APPROVAL;
        let mut dispatch = client();
        dispatch.require_attention().unwrap();
        dispatch.prompt(Prompt(prompt.into())).unwrap();
        assert_eq!(peer.receipt(), prompt);
        peer.wait_turn(TurnState::RequiresAction);
        let queued = "queued";
        assert_eq!(
            client().prompt(Prompt(queued.into())).unwrap(),
            InputDisposition::Queued
        );
        client()
            .answer_permission(request, PermissionAnswer::Selected(option.into()))
            .unwrap();
        assert_eq!(fixture.received(), queued);
    }
    if capabilities.cancel {
        client().prompt(Prompt(HELD.into())).unwrap();
        assert_eq!(peer.receipt(), HELD);
        peer.wait_turn(TurnState::Running);
        client().cancel().unwrap();
        assert_eq!(peer.receipt(), CANCEL_RECEIPT);
        peer.wait_turn(TurnState::Running);
        client().cancel().unwrap();
        peer.wait_turn(TurnState::Idle(StopReason::Cancelled));
    }
}

fn fixture_spec(family: &str) -> AgentSpec {
    let catalogue = ModelCatalogue::fixture();
    let (model, efforts) = catalogue.models(family).unwrap().next().unwrap();
    AgentSpec::from_parts(family, Some(model), efforts.first().map(String::as_str)).unwrap()
}

fn child_command(entry: &str, root: &Utf8Path) -> Command {
    // Unit tests run concurrently, so fixture environment belongs to a subprocess.
    let mut command = Command::new(env::current_exe().unwrap());
    command
        .args(["--ignored", "--exact", entry, "--nocapture"])
        .env("PATH", format!("{}:{}", root, env::var("PATH").unwrap()));
    command
}

// std has no deadline-aware process or file wait. Child::try_wait is nonblocking:
// https://doc.rust-lang.org/1.89.0/std/process/struct.Child.html#method.try_wait
fn wait_for<T>(
    label: &str,
    timeout: Duration,
    mut poll: impl FnMut() -> Result<Option<T>>,
) -> Result<T> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .context("deadline overflow")?;
    loop {
        if let Some(value) = poll()? {
            return Ok(value);
        }
        ensure!(Instant::now() < deadline, "timed out waiting for {label}");
        thread::sleep(Duration::from_millis(10));
    }
}

fn take_receipt(root: &Utf8Path, timeout: Duration) -> Result<String> {
    let path = root.join("receipt");
    let text = wait_for("fake CLI receipt", timeout, || {
        match fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    })?;
    // Consume each publication so repeated identical prompts need their own CLI receipt.
    fs::remove_file(path)?;
    Ok(text)
}
