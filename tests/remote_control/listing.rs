use super::*;
use std::process::Command;
use std::thread;
use std::time::Instant;

#[test]
fn invalid_remote_configuration_fails_before_binding() {
    for (config, script, error) in [
        (None, "#!/bin/sh\nexit 99\n", "remote.relay_node must be"),
        (
            Some(
                "remote:\n  wake_hosts: { spare: { mac: '01:23:45:67:89:AB', broadcast: 192.168.1.255 } }\n  mcp: {resource: 'https://localhost/mcp', issuers: {}, port: 0}\n",
            ),
            "#!/bin/sh\nexit 99\n",
            "remote.wake_hosts requires remote.relay_node",
        ),
        (
            Some("remote: { relay_node: relay }"),
            "#!/bin/sh\necho 'no local daemon' >&2\nexit 1\n",
            "no local daemon",
        ),
        (
            Some("remote: { relay_node: relay }"),
            "#!/bin/sh\necho not-an-ip\n",
            "parse tailscale ip -4",
        ),
    ] {
        let env = TestEnv::new("remote-before-binding");
        if let Some(config) = config {
            atomic_write(env.home.join(".niles/config.yaml"), config).unwrap();
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

#[test]
fn mcp_can_run_alone_or_beside_the_separate_relay_listener() {
    for relay in [false, true] {
        let env = TestEnv::new("remote-mcp");
        let relay_config = if relay {
            stub_tailscale(&env);
            "  relay_node: relay\n"
        } else {
            write_executable(&env.bin.join("tailscale"), "#!/bin/sh\nexit 99\n");
            ""
        };
        atomic_write(
            env.home.join(".niles/config.yaml"),
            format!("remote:\n{relay_config}  mcp:\n    port: 0\n    resource: https://localhost/mcp\n    issuers: {{}}\n"),
        ).unwrap();
        let mut child = env
            .niles(&env.root, &["remote", "--port", "0"])
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut diagnostics = BufReader::new(child.stderr.take().unwrap());
        let mut addresses = Vec::new();
        for _ in 0..=usize::from(relay) {
            let mut line = String::new();
            diagnostics.read_line(&mut line).unwrap();
            addresses.push(line);
        }
        let mut server = Server {
            child,
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
            server.status("GET", "/api/projects", "", 200);
            server.status("GET", "/mcp", "", 404);
        }
    }
}
