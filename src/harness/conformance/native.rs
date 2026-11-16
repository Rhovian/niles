use super::*;
use crate::agents::{InvocationDefaults, ModelRoster};
use crate::harness::native::NativeTui;
use crate::telemetry::fixtures;
use crate::tmux::TmuxTarget;
use crate::{agent_window, agents, store::atomic_write, test_support};
use camino::Utf8PathBuf;
use serde_json::Value;
use std::{env, fs, process::Command};

// Like the existing real-tmux fixtures, the fake accepts bracketed paste without advertising
// paste mode. Perl's record separator preserves literal newlines until the submit CR.
const CLI: &str = r#"#!/bin/sh
for arg do :; done
printf '%s' "$arg" > "$RECEIPT"
stty -icanon -icrnl
tmux wait-for -S conformance-ready
printf 'ready\n'
exec perl -e '$/ = "\r"; while (<STDIN>) { s/^\e\[200~//; s/\e\[201~\r$//; open my $f, ">", $ENV{RECEIPT} or die $!; print $f $_; close $f or die $!; print "\nreceived\n"; }'
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
        // tmux 3.4 wait-for retains a signal until consumed, so startup needs no polling.
        // https://github.com/tmux/tmux/blob/3.4/cmd-wait-for.c
        tmux_output(&["wait-for", "conformance-ready"]);
        Self::Adapter {
            target: TmuxTarget::pane(&self.pane).unwrap(),
        }
    }
    fn received(&self) -> String {
        fs::read_to_string(self.root.join("receipt")).unwrap()
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
        self.start("restart")
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
    let output = Command::new(env::current_exe().unwrap())
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
        .output()
        .unwrap();
    let cleanup = Command::new("tmux")
        .args(["-S", &socket, "kill-server"])
        .output()
        .unwrap();
    assert!(cleanup.status.success(), "{cleanup:?}");
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "{output:?}");
}
