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
case "$1" in
  has-session) exit 0 ;;
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
        json!({"id":"session", "agent":"claude:opus:high",
        "created_at":"2026-01-02T03:04:05Z", "workspace":env.root,"brief":dir.join("agent.md")})
        .to_string(),
    )
    .unwrap();
    common::write_worker(&env.root, "live", "api:w", None, b"");
    common::write_worker(&env.root, "lost", "api:gone", None, b"");
    let mut server = Server(
        env.niles(&env.root, &["remote", "--port", "0"])
            .env("TMUX_PROJECT_TAG", &env.root)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    BufReader::new(server.0.stderr.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let addr: std::net::SocketAddr = line
        .strip_prefix("remote listening on ")
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(addr.ip(), std::net::Ipv4Addr::LOCALHOST);
    let get = |login: &str| {
        let mut socket = std::net::TcpStream::connect(addr).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        write!(
            socket,
            "GET /api/sessions HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{login}\r\n"
        )
        .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        response
    };
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
}
