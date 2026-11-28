use super::*;
use std::process::Command;
use std::thread;
use std::time::Instant;

#[test]
fn invalid_remote_configuration_fails_before_binding() {
    let issuer = issuer::Issuer::start();
    for (config, error) in [
        (None, "configure remote.web or remote.mcp"),
        (
            Some(
                "remote: { web: { bind: 127.0.0.1, resource: 'https://localhost/wrong' } }".into(),
            ),
            "remote.web.resource must be",
        ),
        (
            Some(issuer.config_with_issuer_path("mismatch")),
            "issuer mismatch",
        ),
        (
            Some(issuer.config_with_issuer_path("insecure")),
            "expected an HTTPS URL",
        ),
    ] {
        let env = TestEnv::new("remote-before-binding");
        if let Some(config) = config {
            atomic_write(env.home.join(".niles/config.yaml"), config).unwrap();
        }
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port().to_string();
        let mut command = env.niles(&env.root, &["remote", "--port", &port]);
        issuer.trust(&mut command);
        let output = command.output().unwrap();
        assert_failure_contains(error, &output, error);
        assert!(!stderr_of(&output).contains("Address already in use"));
        assert!(!stderr_of(&output).contains("remote listening"));
    }
}

#[test]
fn fatal_server_error_exits_while_a_blocking_request_is_running() {
    // The push watcher can invoke tmux as soon as the server starts.
    let tmux = TMUX.replace(
        "kill-session)",
        "kill-session) echo $$ > \"$HOME/blocked-pid\"; exec sleep 30;",
    );
    let env = TestEnv::with_tmux("remote-fatal", &tmux);
    atomic_write(marker(&env, "running"), "").unwrap();
    assert_command_success(
        "enable push",
        &env.run(&["remote", "init", "--subject", "mailto:test@example.com"]),
    );
    let mut server = Server::new(&env);
    let pid = env.home.join("blocked-pid");
    let _socket = server.socket("DELETE", "/api/projects/api/session", "");
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
    assert!(until(&mut || pid.exists()), "request never entered tmux");
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
    assert_command_success("release blocking tmux", &output);
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

#[test]
fn mcp_can_run_alone_or_beside_the_separate_relay_listener() {
    for relay in [false, true] {
        let env = TestEnv::new("remote-mcp");
        let mut command = env.niles(&env.root, &["remote", "--port", "0"]);
        let issuer = relay.then(issuer::Issuer::start);
        let web_config = issuer.as_ref().map_or_else(String::new, |issuer| {
            issuer.trust(&mut command);
            issuer.config()
        });
        atomic_write(
            env.home.join(".niles/config.yaml"),
            format!(
                "remote:\n{web_config}  mcp:\n    port: 0\n    resource: https://localhost/mcp\n"
            ),
        )
        .unwrap();
        let mut child = command.stderr(Stdio::piped()).spawn().unwrap();
        let mut diagnostics = BufReader::new(child.stderr.take().unwrap());
        let mut addresses = Vec::new();
        for _ in 0..=usize::from(relay) {
            let mut line = String::new();
            diagnostics.read_line(&mut line).unwrap();
            addresses.push(line);
        }
        let mut server = Server {
            child,
            issuer,
            address: String::new(),
            _diagnostics: diagnostics,
        };
        let address = |prefix| {
            addresses
                .iter()
                .find_map(|line| line.strip_prefix(prefix))
                .map(str::trim)
        };
        server.address = address("remote MCP listening on ").unwrap().to_owned();
        assert!(server.address.starts_with("127.0.0.1:"));
        server.status("GET", "/mcp", "", 401);
        server.status("GET", "/api/projects", "", 404);
        if relay {
            let relay_address = address("remote listening on ").unwrap();
            assert_ne!(server.address, relay_address);
            server.address = relay_address.to_owned();
            let exchange = server.request_with(
                "POST",
                "/api/auth/token",
                "",
                r#"{"code":"code","code_verifier":"verifier"}"#,
            );
            assert!(exchange.starts_with("HTTP/1.1 400"), "{exchange}");
            server.status("GET", "/api/projects", "", 200);
            server.status("GET", "/mcp", "", 404);
        }
    }
}

#[test]
fn opaque_tokens_are_exchanged_and_introspected_for_one_host() {
    let env = TestEnv::with_tmux("opaque-remote", TMUX);
    let server = Server::with_issuer(&env, issuer::Issuer::opaque());
    for (grant, status, refresh) in [
        (
            json!({"code":"valid-code", "code_verifier":"v".repeat(43)}),
            200,
            Some("first-refresh"),
        ),
        (
            json!({"refresh_token":"first-refresh"}),
            200,
            Some("rotated-refresh"),
        ),
        (
            json!({"code":"used-code", "code_verifier":"v".repeat(43)}),
            400,
            None,
        ),
        (json!({"refresh_token":"used-refresh"}), 400, None),
    ] {
        let response = server.request_with("POST", "/api/auth/token", "", &grant.to_string());
        assert!(
            response.starts_with(&format!("HTTP/1.1 {status}")),
            "{response}"
        );
        if let Some(refresh_token) = refresh {
            assert!(response.contains("cache-control: no-store"));
            let (_, body) = response.split_once("\r\n\r\n").unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(body).unwrap(),
                json!({"access_token":"valid-opaque", "token_type":"Bearer", "expires_in":300, "refresh_token":refresh_token})
            );
        }
    }
    for (token, status) in [
        ("valid-opaque", 200),
        ("single-audience", 200),
        ("wrong-resource", 401),
        ("wrong-client", 401),
        ("two-resources", 401),
        ("foreign-issuer", 401),
        ("unknown-subject", 403),
        ("expired", 401),
        ("future", 401),
        ("wrong-type", 401),
        ("empty-subject", 401),
        ("inactive", 401),
    ] {
        let response = server.request_with(
            "GET",
            "/api/projects",
            &format!("Authorization: Bearer {token}\r\n"),
            "",
        );
        assert!(
            response.starts_with(&format!("HTTP/1.1 {status}")),
            "{token}: {response}"
        );
    }
}
