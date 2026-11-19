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
