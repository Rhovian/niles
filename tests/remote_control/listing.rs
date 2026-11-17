use super::*;
use std::process::Command;
use std::thread;
use std::time::Instant;

#[test]
fn invalid_remote_configuration_fails_before_binding() {
    for (relay, script, error) in [
        (false, "#!/bin/sh\nexit 99\n", "remote.relay_node must be"),
        (
            true,
            "#!/bin/sh\necho 'no local daemon' >&2\nexit 1\n",
            "no local daemon",
        ),
        (true, "#!/bin/sh\necho not-an-ip\n", "parse tailscale ip -4"),
    ] {
        let env = TestEnv::new("remote-before-binding");
        if relay {
            atomic_write(
                env.home.join(".niles/config.yaml"),
                "remote: { relay_node: relay }",
            )
            .unwrap();
        }
        write_executable(&env.bin.join("tailscale"), script);
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port().to_string();
        let output = env.run(&["remote", "--port", &port]);
        assert_failure_contains(error, &output, error);
        assert!(!stderr_of(&output).contains("Address already in use"));
        assert!(!stderr_of(&output).contains("remote listening"));
    }
}

#[test]
fn server_binds_tailscale_address_and_reads_registered_live_sessions() {
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
    let dir = common::state_dir(&env.root).join("sessions/20260102T030405000000000Z");
    fs::create_dir_all(&dir).unwrap();
    atomic_write(
        dir.join("session.json"),
        json!({"id":"20260102T030405000000000Z", "entry_skill":"lead", "agent":"claude:opus:high",
        "created_at":"2026-01-02T03:04:05Z", "workspace":env.root,"brief":dir.join("agent.md")})
        .to_string(),
    )
    .unwrap();
    let live = common::write_worker(&env.root, "live", "api:w", None, b"");
    let transcript = env.root.join("working-worker");
    fs::create_dir(&transcript).unwrap();
    atomic_write(transcript.join("session.jsonl"), "{\"type\":\"message\",\"timestamp\":\"2026-01-02T03:04:05Z\",\"message\":{\"role\":\"user\"}}\n").unwrap();
    let worker_meta = live.join("meta.json");
    edit_json(&worker_meta, |worker| {
        worker["model"] = json!("worker-model");
        worker["session_link"] = json!({"family": "pi", "session_dir": transcript});
    });
    common::write_worker(&env.root, "lost", "api:gone", None, b"");
    let broken = common::write_worker(&env.root, "broken", "api:broken", None, b"");
    atomic_write(broken.join("meta.json"), "{").unwrap();
    let server = Server::new(&env);
    let addr: SocketAddr = server.address.parse().unwrap();
    assert_eq!(addr.ip(), Ipv4Addr::LOCALHOST);
    let request = |method: &str, path: &str, login: &str, body: &str| {
        server.request_with(method, path, login, body)
    };
    let get = |login| request("GET", "/api/sessions", login, "");
    assert!(get("").starts_with("HTTP/1.1 403"));
    for (marker, status) in [
        ("whois-other", 403),
        ("whois-fail", 500),
        ("whois-bad", 500),
        ("whois-empty", 500),
    ] {
        atomic_write(env.home.join(marker), "").unwrap();
        let response =
            get("Tailscale-User-Login: alice@example.com\r\nX-Forwarded-For: 100.64.0.1\r\n");
        assert!(
            response.starts_with(&format!("HTTP/1.1 {status}")),
            "{response}"
        );
        assert!(response.contains("cache-control: no-store"));
        assert!(!response.contains("daemon unavailable"));
        fs::remove_file(env.home.join(marker)).unwrap();
    }
    assert!(get("Tailscale-User-Login: other@example.net\r\n").starts_with("HTTP/1.1 200"));
    let response = get("Tailscale-User-Login: alice@example.com\r\n");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("cache-control: no-store"));
    let body: serde_json::Value =
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        body,
        json!({"api_version":1,"sessions":[{"id":"api:20260102T030405000000000Z","project":"api",
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
        json_field(&projects, "/projects/0/workers/0/model"),
        "worker-model"
    );
    let workers = projects
        .pointer("/projects/0/workers")
        .unwrap()
        .as_array()
        .unwrap();
    let broken = workers
        .iter()
        .find(|worker| worker["id"] == "broken")
        .unwrap();
    assert_eq!(broken["error"], "worker metadata is unreadable");
    server.status("GET", "/api/telemetry", "", 200);
    let login = "Tailscale-User-Login: alice@example.com\r\n";
    let endpoint = "/api/sessions/api%3A20260102T030405000000000Z/message";
    let post = |path: &str, login: &str, body: &str| request("POST", path, login, body);
    let before_auth = env.tmux_log();
    assert!(post(endpoint, "", r#"{"text":"hi"}"#).starts_with("HTTP/1.1 403"));
    assert!(
        post(
            endpoint,
            "Tailscale-User-Login: alice@example.com\r\nTailscale-User-Login: bob@example.com\r\n",
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
    edit_json(&meta_path, |meta| {
        meta["lead_pane"] = json!("%7");
        meta["entry_skill"] = json!("solo");
    });
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
    let log = env.tmux_log();
    assert!(
        log.contains(&format!("send-keys -t %7 -l \u{1b}[200~{text}\u{1b}[201~")),
        "{log}"
    );
    assert!(log.contains("send-keys -t %7 C-m"));
    assert!(!env.root.join("nope").exists());
    atomic_write(format!("{}.fail", env.tmux_log.display()), "").unwrap();
    let failed = post(endpoint, login, r#"{"text":"fail"}"#);
    assert!(failed.starts_with("HTTP/1.1 500"), "{failed}");
    assert!(failed.contains("cache-control: no-store"));
    assert!(!failed.contains("private tmux failure"));
    assert_eq!(failed.split_once("\r\n\r\n").unwrap().1, "");
    drop(server);
    assert_command_success(
        "enable push",
        &env.run(&["remote", "init", "--subject", "mailto:test@example.com"]),
    );
    atomic_write(&meta_path, "{").unwrap();
    fs::remove_file(env.home.join(".niles/projects/api")).unwrap();
    let mut server = Server::new(&env);
    thread::sleep(Duration::from_millis(1200));
    assert!(
        server.child.try_wait().unwrap().is_none(),
        "corrupt lead stopped the push watcher/server"
    );
    let projects = server.status("GET", "/api/projects", "", 200);
    let projects: serde_json::Value =
        serde_json::from_str(projects.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        json_field(&projects, "/projects/0/session_error"),
        "session metadata is unreadable"
    );
}

#[test]
fn fatal_server_error_exits_while_a_blocking_request_is_running() {
    let env = TestEnv::with_tmux("remote-fatal", TMUX);
    atomic_write(marker(&env, "running"), "").unwrap();
    assert_command_success(
        "enable push",
        &env.run(&["remote", "init", "--subject", "mailto:test@example.com"]),
    );
    let mut server = Server::new(&env);
    let pid = env.home.join("blocked-pid");
    write_executable(
        &env.bin.join("tailscale"),
        r#"#!/bin/sh
echo $$ > "$HOME/blocked-pid"
exec sleep 30
"#,
    );
    let _socket = server.socket("GET", "/api/projects", "");
    let until = |condition: &mut dyn FnMut() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    };
    assert!(until(&mut || pid.exists()), "request never entered WhoIs");
    atomic_write(
        env.home.join(".niles/projects/broken"),
        "invalid registry link",
    )
    .unwrap();
    let exited = until(&mut || server.child.try_wait().unwrap().is_some());
    let output = Command::new("kill")
        .args(["-TERM", fs::read_to_string(pid).unwrap().trim()])
        .output()
        .unwrap();
    assert_command_success("release blocking WhoIs", &output);
    assert!(exited, "fatal error left the server process alive");
    assert!(!server.child.wait().unwrap().success());
    let mut diagnostics = String::new();
    server
        ._diagnostics
        .read_to_string(&mut diagnostics)
        .unwrap();
    assert!(
        diagnostics.contains("watch Web Push notifications"),
        "{diagnostics}"
    );
    assert!(
        diagnostics.contains("invalid registry link"),
        "{diagnostics}"
    );
}
