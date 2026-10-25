#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::{TestEnv, stderr_of};

#[cfg(not(feature = "remote"))]
#[test]
fn disabled_remote_is_a_recognized_command_with_install_instructions() {
    let env = TestEnv::new("remote-disabled");
    let output = env.run(&["remote"]);
    assert!(!output.status.success());
    assert!(stderr_of(&output).contains("built without remote; reinstall with --features remote"));
    assert!(output.stdout.is_empty());
}

#[cfg(feature = "remote")]
#[test]
fn empty_allowlist_refuses_startup_before_binding() {
    let env = TestEnv::new("remote-empty-users");
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port().to_string();
    let output = env.run(&["remote", "--port", &port]);
    assert!(!output.status.success());
    assert!(stderr_of(&output).contains("remote.allowed_users must not be empty"));
    assert!(!stderr_of(&output).contains("Address already in use"));
}

#[cfg(feature = "remote")]
#[test]
fn server_binds_loopback_and_reads_registered_live_sessions() {
    use serde_json::json;
    use std::{
        fs,
        io::{BufRead, BufReader, Read, Write},
        process::{Child, Stdio},
    };
    struct Server(Child);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let env = TestEnv::with_tmux(
        "remote-live",
        r#"#!/bin/sh
[ "$1" != -u ] || shift
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  capture-pane) cat "$TMUX_LOG.pane" 2>/dev/null; exit 0 ;;
  send-keys)
    if [ -f "$TMUX_LOG.fail" ]; then echo 'private tmux failure' >&2; exit 1; fi
    if [ "$4" = "-l" ]; then printf '%s\n' "$5" >> "$TMUX_LOG.pane";
    else printf 'submitted\n' >> "$TMUX_LOG.pane"; fi ;;
  has-session) exit 0 ;;
  display-message) printf '0\n' ;;
  display) printf '%s\n' "$TMUX_PROJECT_TAG" ;;
  list-windows)
    case "$*" in
      *'#{pane_dead}'*) printf 'niles\t0\nw\t0\n' ;;
      *) printf '0\tniles\n1\tw\n' ;;
    esac ;;
esac
"#,
    );
    let niles = env.home.join(".niles");
    fs::create_dir_all(niles.join("projects")).unwrap();
    std::os::unix::fs::symlink(&env.root, niles.join("projects/api")).unwrap();
    fs::write(
        niles.join("config.yaml"),
        "remote: { allowed_users: ['alice@example.com'] }",
    )
    .unwrap();
    let dir = common::state_dir(&env.root).join("sessions/session");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("session.json"),
        json!({"id":"session", "entry_skill":"lead", "agent":"claude:opus:high",
        "created_at":"2026-01-02T03:04:05Z", "workspace":env.root,"brief":dir.join("agent.md")})
        .to_string(),
    )
    .unwrap();
    let live = common::write_worker(&env.root, "live", "api:w", None, b"");
    let transcript = env.root.join("working-worker");
    fs::create_dir(&transcript).unwrap();
    fs::write(transcript.join("session.jsonl"), "{\"type\":\"message\",\"timestamp\":\"2026-01-02T03:04:05Z\",\"message\":{\"role\":\"user\"}}\n").unwrap();
    let worker_meta = live.join("meta.json");
    let mut worker: serde_json::Value =
        serde_json::from_slice(&fs::read(&worker_meta).unwrap()).unwrap();
    worker["model"] = json!("worker-model");
    worker["session_link"] = json!({"family": "pi", "session_dir": transcript});
    fs::write(worker_meta, worker.to_string()).unwrap();
    common::write_worker(&env.root, "lost", "api:gone", None, b"");
    let mut server = Server(
        env.niles(&env.root, &["remote", "--port", "0"])
            .env("TMUX_PROJECT_TAG", &env.root)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    let mut diagnostics = BufReader::new(server.0.stderr.take().unwrap());
    diagnostics.read_line(&mut line).unwrap();
    let addr: std::net::SocketAddr = line
        .strip_prefix("remote listening on ")
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(addr.ip(), std::net::Ipv4Addr::LOCALHOST);
    let request = |method: &str, path: &str, login: &str, body: &str| {
        let mut socket = std::net::TcpStream::connect(addr).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        write!(
            socket,
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{login}\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        response
    };
    let get = |login| request("GET", "/api/sessions", login, "");
    assert!(get("").starts_with("HTTP/1.1 403"));
    let response = get("Tailscale-User-Login: alice@example.com\r\n");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("cache-control: no-store"));
    let body: serde_json::Value =
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        body,
        json!({"api_version":1,"sessions":[{"id":"api:session","project":"api",
        "agent":{"family":"claude","entry_skill":"lead"},"waiting":true,"worker_count":1}]})
    );
    let projects = request(
        "GET",
        "/api/projects",
        "Tailscale-User-Login: alice@example.com\r\n",
        "",
    );
    let projects: serde_json::Value =
        serde_json::from_str(projects.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        projects["projects"][0]["workers"][0]["model"],
        "worker-model"
    );
    let login = "Tailscale-User-Login: alice@example.com\r\n";
    let endpoint = "/api/sessions/api%3Asession/message";
    let post = |path: &str, login: &str, body: &str| request("POST", path, login, body);
    let before_auth = env.tmux_log();
    assert!(post(endpoint, "", r#"{"text":"hi"}"#).starts_with("HTTP/1.1 403"));
    assert!(
        post(
            endpoint,
            "Tailscale-User-Login: mallory@example.com\r\n",
            r#"{"text":"hi"}"#
        )
        .starts_with("HTTP/1.1 403")
    );
    assert!(post(endpoint, "", "not json").starts_with("HTTP/1.1 403"));
    assert_eq!(env.tmux_log(), before_auth);
    assert!(
        post(
            "/api/sessions/%2Fetc%2Fpasswd/message",
            login,
            r#"{"text":"hi"}"#
        )
        .starts_with("HTTP/1.1 404")
    );
    assert!(post(endpoint, login, r#"{"text":"hi"}"#).starts_with("HTTP/1.1 503"));
    assert!(!env.tmux_log().contains("send-keys"));
    let meta_path = dir.join("session.json");
    let mut meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&meta_path).unwrap()).unwrap();
    meta["lead_pane"] = json!("%7");
    meta["entry_skill"] = json!("solo");
    fs::write(&meta_path, meta.to_string()).unwrap();
    for invalid in [
        r#"{}"#,
        r#"{"text":5}"#,
        r#"{"text":"   "}"#,
        r#"{"text":""}"#,
        r#"{"text":"x\u001b[201~"}"#,
        r#"{"text":"x\u0000"}"#,
        r#"{"text":"hi","client_id":"old"}"#,
        "not json",
    ] {
        let response = post(endpoint, login, invalid);
        let expected = if invalid == "not json" { "400" } else { "422" };
        assert!(
            response.starts_with(&format!("HTTP/1.1 {expected}")),
            "{response}"
        );
        assert!(response.contains("cache-control: no-store"));
    }
    let oversized = json!({"text":"x".repeat(16 * 1024)}).to_string();
    assert!(post(endpoint, login, &oversized).starts_with("HTTP/1.1 413"));
    assert!(!env.tmux_log().contains("send-keys"));
    let text = "literal $(touch nope); \"quoted\"\nsecond line 🦀";
    let sent = post(endpoint, login, &json!({"text":text}).to_string());
    assert!(sent.starts_with("HTTP/1.1 200"), "{sent}");
    assert!(sent.contains("cache-control: no-store"));
    assert!(!sent.contains("access-control-allow-origin"));
    let log = env.tmux_log();
    assert!(
        log.contains(&format!("send-keys -t %7 -l \u{1b}[200~{text}\u{1b}[201~")),
        "{log}"
    );
    assert!(log.contains("send-keys -t %7 C-m"));
    assert!(!env.root.join("nope").exists());
    fs::write(format!("{}.fail", env.tmux_log.display()), "").unwrap();
    let failed = post(endpoint, login, r#"{"text":"fail"}"#);
    assert!(failed.starts_with("HTTP/1.1 500"), "{failed}");
    assert!(failed.contains("cache-control: no-store"));
    assert!(!failed.contains("private tmux failure"));
    assert_eq!(failed.split_once("\r\n\r\n").unwrap().1, "");
}

#[cfg(feature = "remote")]
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
