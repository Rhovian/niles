#![cfg(feature = "remote")]
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
use common::*;
use serde_json::json;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, Stdio};
use std::time::Duration;

mod common;
#[path = "remote_control/listing.rs"]
mod listing;

const TMUX: &str = r#"#!/bin/sh
[ "$1" != -u ] || shift
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
has-session) [ -f "$TMUX_LOG.running" ] || { echo "can't find session" >&2; exit 1; } ;;
display) if [ -f "$TMUX_LOG.foreign" ]; then echo /foreign; else printf '%s\n' "$TMUX_PROJECT_TAG"; fi ;;
list-windows) case "$*" in
  *' -a '*) exit 0 ;;
  *'#{pane_dead}'*)
    if [ -f "$TMUX_LOG.lead-absent" ]; then printf 'w\t0\n';
    elif [ -f "$TMUX_LOG.lead-exited" ]; then printf 'niles\t1\nw\t0\n';
    else printf 'niles\t0\nw\t0\n'; fi ;;
  *) printf '0\tniles\n1\tw\n' ;;
  esac ;;
new-session) touch "$TMUX_LOG.running"; if [ -f "$TMUX_LOG.race" ]; then echo duplicate >&2; exit 1; fi ;;
kill-session) rm "$TMUX_LOG.running"; if [ -f "$TMUX_LOG.stop-race" ]; then echo vanished >&2; exit 1; fi ;;
capture-pane) cat "$TMUX_LOG.pane" 2>/dev/null; exit 0 ;;
send-keys) printf '%s\n' "$*" >> "$TMUX_LOG.pane" ;;
esac
"#;
struct Server {
    child: Child,
    address: String,
    _diagnostics: BufReader<ChildStderr>,
}
impl Server {
    fn new(env: &TestEnv) -> Self {
        common::stub_tailscale(env);
        atomic_write(&env.tmux_log, "").unwrap();
        register_project(env, "api", &env.root);
        atomic_write(
            env.home.join(".niles/config.yaml"),
            "remote: { relay_node: relay }",
        )
        .unwrap();
        // Workspace selection must not affect this multi-project service or its close children.
        let mut child = env
            .niles(&env.root, &["remote", "--port", "0"])
            .args(["--setup", "workspace:tdd"])
            .env("NILES_SETUP", "invalid:selection")
            .env("TMUX_PROJECT_TAG", &env.root)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        let mut diagnostics = BufReader::new(child.stderr.take().unwrap());
        diagnostics.read_line(&mut line).unwrap();
        Self {
            child,
            address: line
                .strip_prefix("remote listening on ")
                .unwrap()
                .trim()
                .to_owned(),
            _diagnostics: diagnostics,
        }
    }
    fn socket(&self, method: &str, path: &str, body: &str) -> TcpStream {
        self.socket_with(method, path, "Tailscale-User-Login: alice\r\n", body)
    }
    fn socket_with(&self, method: &str, path: &str, headers: &str, body: &str) -> TcpStream {
        let mut socket = TcpStream::connect(&self.address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(socket, "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        socket
    }
    fn request(&self, method: &str, path: &str, body: &str) -> String {
        self.request_with(method, path, "Tailscale-User-Login: alice\r\n", body)
    }
    fn request_with(&self, method: &str, path: &str, headers: &str, body: &str) -> String {
        let mut result = String::new();
        self.socket_with(method, path, headers, body)
            .read_to_string(&mut result)
            .unwrap();
        assert!(!result.contains("access-control-allow-origin"));
        result
    }
    fn status(&self, method: &str, path: &str, body: &str, expected: u16) -> String {
        let response = self.request(method, path, body);
        assert!(
            response.starts_with(&format!("HTTP/1.1 {expected}")),
            "{method} {path}: {response}"
        );
        response
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if self.child.try_wait().unwrap().is_none() {
            self.child.kill().unwrap();
        }
        self.child.wait().unwrap();
    }
}

fn marker(env: &TestEnv, name: &str) -> PathBuf {
    env.tmux_log.with_extension(format!("log.{name}"))
}

#[test]
fn worker_chat_uses_its_transcript_and_live_window_only() {
    let env = TestEnv::with_tmux("worker-chat", TMUX);
    atomic_write(marker(&env, "running"), "").unwrap();
    let worker = write_worker(&env.root, "worker", "api:w", None, b"");
    let broken = write_worker(&env.root, "broken", "api:broken", None, b"");
    atomic_write(broken.join("meta.json"), "{").unwrap();
    write_worker(&env.root, "lost", "api:gone", None, b"");
    let transcript = env.root.join("transcript");
    fs::create_dir(&transcript).unwrap();
    atomic_write(transcript.join("001.jsonl"), format!("{}\n", json!({"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"worker says hi"}]}}))).unwrap();
    edit_json(worker.join("meta.json"), |meta| {
        meta["session_link"] = json!({"family":"pi","session_dir":transcript});
    });
    let server = Server::new(&env);
    let mut stream =
        BufReader::new(server.socket("GET", "/api/projects/api/workers/worker/events", ""));
    let mut received = String::new();
    loop {
        let mut line = String::new();
        stream.read_line(&mut line).unwrap();
        received.push_str(&line);
        if line.contains("worker says hi") {
            break;
        }
    }
    assert!(received.contains("snapshot"), "{received}");
    assert!(received.contains("api/worker:0"));
    server.status(
        "POST",
        "/api/projects/api/workers/worker/message",
        r#"{"text":"hello"}"#,
        200,
    );
    assert!(
        env.tmux_log().contains("send-keys -t =api:=w"),
        "{}",
        env.tmux_log()
    );
    server.status(
        "POST",
        "/api/projects/api/workers/lost/message",
        r#"{"text":"hello"}"#,
        503,
    );
    for id in ["unknown", "%2E%2E", "%2Fetc%2Fpasswd", "--all"] {
        server.status(
            "POST",
            &format!("/api/projects/api/workers/{id}/message"),
            r#"{"text":"hi"}"#,
            404,
        );
        server.status(
            "DELETE",
            &format!("/api/projects/api/workers/{id}"),
            "",
            404,
        );
    }
    server.status("GET", "/api/projects/api/workers/unknown/events", "", 404);
}

#[test]
fn start_checks_manifest_foreign_tag_idempotence_and_duplicate_race() {
    let env = TestEnv::with_tmux("remote-start", TMUX);
    let server = Server::new(&env);
    server.status("PUT", "/api/projects/unknown/session", "", 404);
    register_project(&env, "missing", &env.root.join("missing"));
    assert!(
        server
            .status("PUT", "/api/projects/missing/session", "", 409)
            .contains("Project directory missing")
    );
    assert!(
        server
            .status("PUT", "/api/projects/api/session", "", 409)
            .contains("No saved manifest; start once at the desk")
    );
    assert!(!env.tmux_log().contains("new-session"));
    write_workspace_manifest(&env.root, "claude", "codex", "claude", "claude");
    atomic_write(marker(&env, "running"), "").unwrap();
    atomic_write(marker(&env, "foreign"), "").unwrap();
    assert!(
        server
            .status("PUT", "/api/projects/api/session", "", 409)
            .contains("Tmux session name held by a foreign session")
    );
    server.status("DELETE", "/api/projects/api/session", "", 404);
    assert!(!env.tmux_log().contains("kill-session"));
    fs::remove_file(marker(&env, "foreign")).unwrap();
    fs::remove_file(marker(&env, "running")).unwrap();
    server.status("PUT", "/api/projects/api/session", "", 200);
    let log = env.tmux_log();
    assert!(log.contains("start --keep-skills"), "{log}");
    assert!(log.contains("env -u NILES_SETUP"));
    assert!(!log.contains("--setup"));
    server.status("PUT", "/api/projects/api/session", "", 200);
    assert_eq!(env.tmux_log().matches("new-session").count(), 1);
    fs::remove_file(marker(&env, "running")).unwrap();
    atomic_write(marker(&env, "race"), "").unwrap();
    server.status("PUT", "/api/projects/api/session", "", 200);
    for path in ["/api/projects/api/session", "/api/projects/api/workers/w"] {
        server.status("POST", path, "", 405);
    }
    for (state, command) in [
        ("lead-absent", "new-window"),
        ("lead-exited", "respawn-window"),
    ] {
        atomic_write(marker(&env, state), "").unwrap();
        server.status("PUT", "/api/projects/api/session", "", 200);
        assert!(
            env.tmux_log()
                .lines()
                .any(|line| line.starts_with(command) && line.contains("start --keep-skills"))
        );
        fs::remove_file(marker(&env, state)).unwrap();
    }
    // A malformed manifest must propagate instead of launching with defaults.
    atomic_write(
        common::state_dir(&env.root).join("manifest.yaml"),
        "lead: [broken",
    )
    .unwrap();
    server.status("PUT", "/api/projects/api/session", "", 500);
    assert_eq!(env.tmux_log().matches("new-session").count(), 2);
}

#[test]
fn close_and_stop_archive_workers_before_killing_the_session() {
    let env = TestEnv::with_tmux("remote-stop", TMUX);
    atomic_write(marker(&env, "running"), "").unwrap();
    let live = write_worker(&env.root, "live", "api:w", None, b"");
    let lost = write_worker(&env.root, "lost", "api:gone", None, b"");
    let server = Server::new(&env);
    server.status("DELETE", "/api/projects/api/workers/lost", "", 200);
    assert!(!lost.join("meta.json").exists());
    server.status("DELETE", "/api/projects/api/workers/lost", "", 404);
    atomic_write(marker(&env, "stop-race"), "").unwrap();
    server.status("DELETE", "/api/projects/api/session", "", 200);
    assert!(!live.join("meta.json").exists());
    let log = env.tmux_log();
    assert!(
        log.find("kill-window").unwrap() < log.find("kill-session").unwrap(),
        "{log}"
    );
    assert!(
        fs::read_dir(common::state_dir(&env.root).join("worker/archive"))
            .unwrap()
            .count()
            >= 2
    );
    server.status("DELETE", "/api/projects/api/session", "", 404);
}

#[test]
fn phone_server_uses_each_project_default_despite_workspace_selection() {
    let env = TestEnv::with_tmux("remote-project-setups", TMUX);
    let other = env.root.join("other");
    fs::create_dir_all(&other).unwrap();
    for root in [&env.root, &other] {
        write_workspace_manifest(root, "claude", "codex", "claude", "claude");
    }
    let named = env.root.join(".niles/setups/tdd.yaml");
    fs::create_dir_all(named.parent().unwrap()).unwrap();
    atomic_write(named, "invalid selected setup").unwrap();
    let server = Server::new(&env);
    register_project(&env, "other", &other);
    for project in ["api", "other"] {
        server.status("PUT", &format!("/api/projects/{project}/session"), "", 200);
        fs::remove_file(marker(&env, "running")).unwrap();
    }
    let log = env.tmux_log();
    assert_eq!(log.matches("new-session").count(), 2);
    assert!(!log.contains("NILES_SETUP="));
    assert!(!log.contains("--setup"));
    write_worker(&other, "worker", "other:w", None, b"");
    server.status("DELETE", "/api/projects/other/workers/worker", "", 200);
}

#[test]
fn remote_init_creates_private_keys_once_and_refuses_invalid_contact() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let env = TestEnv::new("remote-push-init");
    let key = env.home.join(".niles/remote/vapid.json");
    let invalid = env.run(&["remote", "init", "--subject", "file:///private"]);
    assert!(!invalid.status.success());
    assert!(!key.exists());
    let args = ["remote", "init", "--subject", "mailto:operator@example.com"];
    let initialized = env.run(&args);
    assert!(initialized.status.success(), "{}", stderr_of(&initialized));
    assert_eq!(
        fs::metadata(&key).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let before = fs::read(&key).unwrap();
    let repeated = env.run(&args);
    assert!(!repeated.status.success());
    assert_eq!(fs::read(&key).unwrap(), before);
}
