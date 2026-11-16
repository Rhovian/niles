use super::*;
use crate::agents::{InvocationDefaults, ModelRoster};
use crate::harness::native::NativeTui;
use crate::telemetry::fixtures;
use crate::tmux::TmuxTarget;
use crate::{agent_window, agents, store::atomic_write, test_support};
use anyhow::{Context, Result, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;
use std::{
    env, fs,
    fs::File,
    io::ErrorKind,
    process::Command,
    thread,
    time::{Duration, Instant},
};

// Like the existing real-tmux fixtures, the fake accepts bracketed paste without advertising
// paste mode. Perl's record separator preserves literal newlines until the submit CR.
// Delayed startup and receipt publication exercise scheduling gaps after terminal echo.
// Rename publishes a complete receipt: https://perldoc.perl.org/functions/rename
const CLI: &str = r#"#!/bin/sh
set -eu
for arg do :; done
sleep 1
stty -icanon -icrnl
printf '%s' "$arg" > "$RECEIPT.tmp"
mv "$RECEIPT.tmp" "$RECEIPT"
printf 'ready\n'
exec perl -e '$| = 1; $/ = "\r"; while (<STDIN>) { s/^\e\[200~//; s/\e\[201~\r$//; print "\nprocessing\n"; select undef, undef, undef, 0.25; open my $f, ">", "$ENV{RECEIPT}.tmp" or die $!; print $f $_; close $f or die $!; rename "$ENV{RECEIPT}.tmp", $ENV{RECEIPT} or die $!; print "\nreceived\n"; }'
"#;

struct Native {
    family: &'static str,
    root: Utf8PathBuf,
    pane: String,
    body: String,
}
impl Fixture for Native {
    type Adapter = NativeTui;
    fn start(&mut self, first: &str) -> Self::Adapter {
        let brief = self.root.join("brief");
        atomic_write(&brief, first).unwrap();
        let mut invocation = agents::invocation(
            self.family,
            InvocationDefaults::Worker,
            &ModelRoster::builtin().unwrap(),
        )
        .unwrap();
        invocation
            .env
            .push(("PATH".into(), env::var("PATH").unwrap()));
        invocation
            .env
            .push(("RECEIPT".into(), self.root.join("receipt").into_string()));
        let launch = self.root.join("launch");
        atomic_write(
            &launch,
            agent_window::launch_script(&invocation, &brief, &self.root.join("status")),
        )
        .unwrap();
        let output = tmux_output(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            "conformance:",
            &format!("sh '{}'", launch),
        ]);
        self.pane = output.trim().into();
        wait_for("fake CLI startup", Duration::from_secs(30), || {
            Ok(self.root.join("receipt").try_exists()?.then_some(()))
        })
        .unwrap();
        Self::Adapter {
            target: TmuxTarget::pane(&self.pane).unwrap(),
        }
    }
    fn received(&self) -> String {
        take_receipt(&self.root, Duration::from_secs(30)).unwrap()
    }
    fn disposition(&self, _: bool) -> InputDisposition {
        InputDisposition::Unconfirmed
    }
    fn signal(&mut self, signal: Signal) {
        self.body = transcript(self.family, signal).unwrap();
    }
    fn observe(&mut self, _: i64) -> TurnState {
        fixtures::turn(self.family, &self.body).unwrap().state
    }
    fn disconnect(&mut self) {
        tmux_output(&["kill-pane", "-t", &self.pane]);
    }
    fn restart(&mut self) -> Self::Adapter {
        let adapter = self.start("restart");
        assert_eq!(self.received(), "restart");
        adapter
    }
    fn stale_event(&mut self) {
        // Native has no event transport: late events cannot enter through updates().
        let mut adapter = Self::Adapter {
            target: TmuxTarget::pane(&self.pane).unwrap(),
        };
        assert_err_contains(
            adapter.updates().map(|_| ()),
            "no streamed update transport",
        );
    }
    fn session(&self) -> SessionId {
        SessionId::new("native-identity-not-resumable".into()).unwrap()
    }
    fn permission(&self) -> PermissionRequest {
        PermissionRequest {
            id: "permission".into(),
            options: vec!["allow".into()],
        }
    }
    fn auth_path(&self) -> &str {
        "fixture/no-auth"
    }
}

fn transcript(family: &str, signal: Signal) -> anyhow::Result<String> {
    use serde_json::json;
    let (running, message) = match signal {
        Signal::Running => (true, ""),
        Signal::Failure(message) => (false, message),
        Signal::Attention | Signal::Cancelled => anyhow::bail!("native declares these unavailable"),
    };
    let stamp = at(1000).to_rfc3339();
    let value = match family {
        "claude" => json!({"type":"assistant", "timestamp":stamp,
            "isApiErrorMessage":!running, "message":{"id":"a", "stop_reason":if running {"tool_use"} else {"refusal"},
            "content":[{"type":"text", "text":message}],
            "usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}),
        "codex" => json!({"type":"event_msg", "timestamp":stamp, "payload":{
            "type":if running {"task_started"} else {"task_complete"},
            "error":if running {Value::Null} else {json!({"message":message})}}}),
        "pi" => json!({"type":"message", "timestamp":stamp, "message":{
            "role":"assistant", "stopReason":if running {"toolUse"} else {"error"}, "errorMessage":message}}),
        _ => anyhow::bail!("every family needs a transcript fixture: {family}"),
    };
    Ok(format!("{value}\n"))
}

fn tmux_output(args: &[&str]) -> String {
    let output = Command::new("tmux").args(args).output().unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
#[ignore = "subprocess entry point; native_families supplies an isolated tmux server and PATH"]
fn native_child() {
    let root = Utf8PathBuf::from(env::var("NILES_CONFORMANCE_ROOT").unwrap());
    for profile in agents::profiles() {
        let dir = root.join(format!("{}-run", profile.id));
        fs::create_dir_all(&dir).unwrap();
        eprintln!(
            "adapter=native-tui family={} protocol=none cli=fixture",
            profile.id
        );
        super::run(&mut Native {
            family: profile.id,
            root: dir,
            pane: String::new(),
            body: String::new(),
        });
    }
}

#[test]
fn native_families() {
    // Environment changes belong in a child: unit tests share a process and run concurrently.
    let root = test_support::temp_dir("harness-conformance");
    for profile in agents::profiles() {
        test_support::write_executable_script(&root.join(profile.binary), CLI);
    }
    let socket = format!("/tmp/nc-{}.sock", uuid::Uuid::new_v4());
    let server = Command::new("tmux")
        .args([
            "-f",
            "/dev/null",
            "-S",
            &socket,
            "new-session",
            "-d",
            "-s",
            "conformance",
            "sleep 600",
        ])
        .output()
        .unwrap();
    assert!(server.status.success(), "{server:?}");
    let mut child = Command::new(env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "harness::conformance::native::native_child",
            "--nocapture",
        ])
        .env("TMUX", format!("{socket},0,0"))
        .env(
            "PATH",
            format!("{}:{}", root.as_str(), env::var("PATH").unwrap()),
        )
        .env("NILES_CONFORMANCE_ROOT", root.as_str())
        .stdout(File::create(root.join("child.stdout")).unwrap())
        .stderr(File::create(root.join("child.stderr")).unwrap())
        .spawn()
        .unwrap();
    let result = wait_for("conformance child", Duration::from_secs(120), || {
        Ok(child.try_wait()?)
    });
    if result.is_err() {
        child.kill().unwrap();
        child.wait().unwrap();
    }
    let cleanup = Command::new("tmux")
        .args(["-S", &socket, "kill-server"])
        .output()
        .unwrap();
    assert!(cleanup.status.success(), "{cleanup:?}");
    let stdout = fs::read_to_string(root.join("child.stdout")).unwrap();
    let stderr = fs::read_to_string(root.join("child.stderr")).unwrap();
    eprintln!("{stderr}");
    assert!(
        result.as_ref().is_ok_and(|status| status.success()),
        "{result:?}\n{stdout}\n{stderr}"
    );
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

#[test]
fn missing_receipt_times_out() {
    let root = test_support::temp_dir("missing-receipt");
    assert_err_contains(
        take_receipt(&root, Duration::ZERO),
        "timed out waiting for fake CLI receipt",
    );
}
