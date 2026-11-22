use super::*;
use crate::agents::{self, AuthSource};
use crate::harness::{acp as headless, launch::Billing};
use crate::{store, test_support};
use camino::Utf8PathBuf;
use std::process::{Child, Stdio};
use std::{env, fs};

const PEER: &str = include_str!("../../../tests/harness_acp.py");
const TIMEOUT: Duration = Duration::from_secs(10);
const PRIMARY: &str = "conformance system prompt";

struct Acp {
    root: Utf8PathBuf,
    spec: AgentSpec,
    running: Option<Runner>,
}

struct Runner {
    dir: Utf8PathBuf,
    child: Child,
}

impl Runner {
    fn stop(mut self) {
        drop(self.child.stdin.take());
        let status = wait_for("ACP runner exit", TIMEOUT, || Ok(self.child.try_wait()?));
        assert!(
            status.as_ref().is_ok_and(|status| status.success()),
            "{status:?}"
        );
    }
}

#[expect(
    clippy::let_underscore_must_use,
    reason = "reap the isolated runner even after a failing assertion"
)]
impl Drop for Runner {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Acp {
    fn launch(&mut self, dir: &Utf8Path, prompt: &str) {
        let profile = self.spec.profile().acp.unwrap();
        let child = child_command("harness::conformance::acp::run_acp_child", &self.root)
            .current_dir(dir)
            .env("HOME", &self.root)
            .env("NILES_CONFORMANCE_DIR", dir)
            .env("NILES_CONFORMANCE_FIRST", prompt)
            .envs(
                profile
                    .remove_env
                    .iter()
                    .map(|key| (key, "fixture-override")),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        self.running = Some(Runner {
            dir: dir.to_owned(),
            child,
        });
    }

    fn dir(&self) -> &Utf8Path {
        &self.running.as_ref().unwrap().dir
    }
}

impl Fixture for Acp {
    type Generation = Utf8PathBuf;

    fn start(&mut self, first: &str) -> Self::Generation {
        let dir = self.root.join("session");
        fs::create_dir(&dir).unwrap();
        let profile = self.spec.profile().acp.unwrap();
        let session = profile
            .adapter
            .configure(&self.spec, PRIMARY, true)
            .unwrap();
        headless::prepare(&dir, self.spec.family(), Billing::Subscription, session).unwrap();
        self.launch(&dir, first);
        dir
    }

    fn restart(&mut self, dir: &Self::Generation, prompt: &str) -> Self::Generation {
        // The last pre-crash prompt finished with MaxTurnRequests. EndTurn on
        // this directory is published only once the replacement finishes loading.
        self.launch(dir, prompt);
        wait_turn(dir, TurnState::Idle(StopReason::EndTurn));
        dir.clone()
    }

    fn connect(dir: &Self::Generation) -> Result<impl Adapter + '_> {
        headless::Headless::connect(dir)
    }

    fn received(&self) -> String {
        let dir = self.dir();
        let receipt = take_receipt(dir, TIMEOUT).unwrap();
        // Startup and resume write EndTurn before any prompt response. Only the
        // peer's chosen stop reason proves that this particular turn ended.
        wait_turn(dir, TurnState::Idle(StopReason::MaxTurnRequests));
        receipt
    }

    fn disposition(&self) -> InputDisposition {
        InputDisposition::Unconfirmed
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            cancel: true,
            resume: true,
            attention: true,
        }
    }

    fn lifecycle_peer(&self) -> Option<&dyn LifecyclePeer> {
        Some(self)
    }

    fn disconnect(&mut self) {
        // A crash leaves the actual socket file behind, so replacement must
        // reclaim it before binding to the same address.
        let mut runner = self.running.take().unwrap();
        runner.child.kill().unwrap();
        runner.child.wait().unwrap();
    }

    fn finish(&mut self) {
        self.running.take().unwrap().stop();
    }

    fn session(&self) -> SessionId {
        headless::session(self.dir()).unwrap()
    }

    fn auth_path(&self) -> &str {
        match self.spec.profile().acp.unwrap().auth {
            AuthSource::Notification(_) => "fixture/auth-status-notification",
            AuthSource::Command(_) => "fixture/pi-auth-check",
        }
    }
}

impl LifecyclePeer for Acp {
    fn set_history_missing(&self, missing: bool) {
        let path = self.dir().join("missing-history");
        if missing {
            store::atomic_write(&path, "").unwrap();
        } else {
            fs::remove_file(path).unwrap();
        }
    }

    fn receipt(&self) -> String {
        take_receipt(self.dir(), TIMEOUT).unwrap()
    }

    fn wait_turn(&self, state: TurnState) {
        wait_turn(self.dir(), state);
    }

    fn transcript(&self) -> String {
        fs::read_to_string(self.dir().join("transcript.jsonl")).unwrap()
    }
}

fn wait_turn(dir: &Utf8Path, expected: TurnState) {
    wait_for(&format!("ACP {expected:?} in {dir}"), TIMEOUT, || {
        Ok(headless::turn(dir)?.filter(|turn| turn.state == expected))
    })
    .unwrap();
}

#[tokio::test]
#[ignore = "subprocess entry point; acp_families supplies the fake peer and stdin"]
async fn run_acp_child() {
    let dir = Utf8PathBuf::from(env::var("NILES_CONFORMANCE_DIR").unwrap());
    headless::run(&dir, env::var("NILES_CONFORMANCE_FIRST").unwrap())
        .await
        .unwrap();
}

#[test]
fn acp_families() {
    // All three peers exercise their launch, auth and load shapes. The queue,
    // attention, cancel and transcript assertions exercise the shared ACP runner.
    for profile in agents::profiles().filter(|profile| profile.acp.is_some()) {
        let root = test_support::temp_dir(profile.id);
        test_support::write_executable_script(&root.join(profile.acp.unwrap().binary), PEER);
        let spec = if profile.id == "pi" {
            test_support::write_executable_script(&root.join("pi"), PEER);
            AgentSpec::from_parts("pi", Some("openai/fixture"), Some("high")).unwrap()
        } else {
            fixture_spec(profile.id)
        };
        eprintln!("adapter=acp family={} protocol=v1 cli=fixture", profile.id);
        let mut fixture = Acp {
            root: root.to_path_buf(),
            spec,
            running: None,
        };
        let coverage = match profile.id {
            "codex" | "pi" => LifecycleCoverage::SharedAcpRunner,
            _ => LifecycleCoverage::Full,
        };
        super::run(&mut fixture, coverage);
    }
}
