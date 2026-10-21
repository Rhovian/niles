#![cfg(feature = "remote")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::{TestEnv, stderr_of};
use std::{fs, process::Command};

#[cfg(target_os = "macos")]
const SERVICE: &str = "Library/LaunchAgents/dev.niles.remote.plist";
#[cfg(target_os = "linux")]
const SERVICE: &str = ".config/systemd/user/niles-remote.service";

struct Setup(TestEnv);
impl Setup {
    fn new() -> Self {
        let env = TestEnv::new("remote-setup");
        fs::create_dir_all(env.root.join("web")).unwrap();
        fs::write(env.root.join("web/index.html"), "shell").unwrap();
        fs::create_dir_all(env.home.join(".niles")).unwrap();
        fs::write(
            env.home.join(".niles/config.yaml"),
            "theme: nord\ntmux: {bindings: true}\nremote: {allowed_users: [old@example.com]}\n",
        )
        .unwrap();
        common::write_executable(
            &env.bin.join("tailscale"),
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$*" in
 version) echo 1.96.2 ;;
 'status --json')
   if [ -n "$TAILNET_STATUS" ]; then printf '%s\n' "$TAILNET_STATUS";
   else echo '{"Self":{"DNSName":"host.tail.ts.net."}}'; fi ;;
 'serve status --json')
   if [ -n "$SERVE_CONFIG" ]; then printf '%s\n' "$SERVE_CONFIG"; else echo null; fi ;;
esac
"#,
        );
        for manager in ["launchctl", "systemctl"] {
            common::write_executable(
                &env.bin.join(manager),
                r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
if [ "$SERVICE_FAIL" = 1 ]; then
 case "$*" in *daemon-reload*) exit 0 ;; esac
 echo 'no service-manager session' >&2; exit 1
fi
"#,
            );
        }
        common::write_executable(
            &env.bin.join("curl"),
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
if [ "$CURL_FAIL" = 1 ]; then echo 'HTTP 403' >&2; exit 22; fi
if [ -n "$CURL_BODY" ]; then printf '%s' "$CURL_BODY";
else printf '{"api_version":1,"sessions":[]}'; fi
printf '\n200\nno-store'
"#,
        );
        Self(env)
    }

    fn command(&self) -> Command {
        self.0.niles(
            &self.0.root,
            &[
                "remote",
                "setup",
                "--web-dir",
                "web",
                "--allow-user",
                "alice@example.com",
            ],
        )
    }

    fn config(&self) -> Vec<u8> {
        fs::read(self.0.home.join(".niles/config.yaml")).unwrap()
    }
}

#[test]
fn remote_setup_installs_service_mounts_and_verifies() {
    let fixture = Setup::new();
    let output = fixture.command().args(["--port", "9123"]).output().unwrap();
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("Verified https://host.tail.ts.net/api/sessions")
    );
    let config = String::from_utf8(fixture.config()).unwrap();
    for setting in [
        "nord",
        "bindings: true",
        "old@example.com",
        "alice@example.com",
    ] {
        assert!(config.contains(setting));
    }
    assert!(fixture.0.home.join(SERVICE).is_file());
    let log = fixture.0.tmux_log();
    assert!(log.contains("serve --bg --https=443 --set-path=/ "));
    assert!(log.contains("serve --bg --https=443 --set-path=/api http://127.0.0.1:9123/api"));
    assert!(log.contains("--write-out"));
    assert!(!log.contains("Tailscale-User-Login"));
    #[cfg(target_os = "macos")]
    assert!(log.contains("bootstrap gui/") && log.contains(SERVICE));
    #[cfg(target_os = "linux")]
    assert!(
        log.contains("--user daemon-reload")
            && log.contains("--user enable --now niles-remote.service")
    );
}

#[test]
fn remote_setup_refuses_existing_service_without_changing_config() {
    let fixture = Setup::new();
    let service = fixture.0.home.join(SERVICE);
    fs::create_dir_all(service.parent().unwrap()).unwrap();
    fs::write(&service, "existing service").unwrap();
    let before = fixture.config();
    let output = fixture.command().output().unwrap();
    assert!(!output.status.success());
    assert!(stderr_of(&output).contains("service already exists"));
    assert_eq!(fixture.config(), before);
    assert_eq!(fs::read_to_string(service).unwrap(), "existing service");
}

#[test]
fn remote_setup_keeps_existing_vapid_identity() {
    let fixture = Setup::new();
    assert!(
        fixture
            .0
            .run(&["remote", "init", "--subject", "mailto:old@example.com"])
            .status
            .success()
    );
    let key = fixture.0.home.join(".niles/remote/vapid.json");
    let before = fs::read(&key).unwrap();
    let output = fixture
        .command()
        .args(["--subject", "mailto:new@example.com"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(fs::read(key).unwrap(), before);
}

#[test]
fn remote_setup_initializes_vapid_when_absent() {
    let fixture = Setup::new();
    let output = fixture
        .command()
        .args(["--subject", "mailto:alice@example.com"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(fixture.0.home.join(".niles/remote/vapid.json").is_file());
}

#[test]
fn remote_setup_failed_activation_removes_unit_and_allows_retry() {
    let fixture = Setup::new();
    let output = fixture.command().env("SERVICE_FAIL", "1").output().unwrap();
    assert!(!output.status.success());
    assert!(stderr_of(&output).contains("no service-manager session"));
    assert!(!fixture.0.home.join(SERVICE).exists());
    assert!(!fixture.0.tmux_log().contains("serve --bg"));
    let output = fixture.command().output().unwrap();
    assert!(output.status.success(), "{}", stderr_of(&output));
}

#[test]
fn remote_setup_verification_failures_warn_and_keep_setup() {
    for (key, value) in [("CURL_FAIL", "1"), ("CURL_BODY", "<html>shell</html>")] {
        let fixture = Setup::new();
        let output = fixture.command().env(key, value).output().unwrap();
        assert!(output.status.success(), "{}", stderr_of(&output));
        assert!(stderr_of(&output).contains("warning: verification failed"));
        assert!(stderr_of(&output).contains(
            "Verify from an allowed tailnet device: curl -i https://host.tail.ts.net/api/sessions"
        ));
        assert!(fixture.0.home.join(SERVICE).is_file());
        assert!(String::from_utf8_lossy(&output.stdout).contains("Add to Home Screen"));
    }
}

#[test]
fn remote_setup_conflicts_and_schema_changes_stop_before_mutation() {
    for config in [
        r#"{"Web":{"host:443":{"Handlers":{"/":{}}}}}"#,
        r#"{"WebV2":{}}"#,
    ] {
        let fixture = Setup::new();
        let before = fixture.config();
        let output = fixture
            .command()
            .env("SERVE_CONFIG", config)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(fixture.config(), before);
        assert!(!fixture.0.home.join(SERVICE).exists());
        assert!(!fixture.0.tmux_log().contains("serve --bg"));
    }
}

#[test]
fn remote_setup_rejects_bad_web_input_with_context() {
    for (path, expected) in [
        ("missing", "resolve web directory missing"),
        ("empty", "must contain index.html"),
    ] {
        let fixture = Setup::new();
        fs::create_dir_all(fixture.0.root.join("empty")).unwrap();
        let output = fixture.0.run(&[
            "remote",
            "setup",
            "--web-dir",
            path,
            "--allow-user",
            "alice@example.com",
        ]);
        assert!(!output.status.success());
        assert!(stderr_of(&output).contains(expected));
        assert!(!fixture.0.home.join(SERVICE).exists());
    }
}

#[test]
fn remote_setup_rejects_missing_or_invalid_host_dns() {
    for status in [
        r#"{}"#,
        r#"{"Self":{"DNSName":""}}"#,
        r#"{"Self":{"DNSName":"bad/name"}}"#,
    ] {
        let fixture = Setup::new();
        let output = fixture
            .command()
            .env("TAILNET_STATUS", status)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(stderr_of(&output).contains("DNS name"));
        assert!(!fixture.0.home.join(SERVICE).exists());
    }
}

#[test]
fn remote_setup_rejects_zero_port_and_init_rejects_port() {
    let fixture = Setup::new();
    let output = fixture.command().args(["--port", "0"]).output().unwrap();
    assert!(!output.status.success());
    assert!(stderr_of(&output).contains("invalid value '0'"));
    for args in [
        vec![
            "remote",
            "init",
            "--subject",
            "mailto:a@example.com",
            "--port",
            "5",
        ],
        vec![
            "remote",
            "--port",
            "5",
            "init",
            "--subject",
            "mailto:a@example.com",
        ],
    ] {
        assert!(!fixture.0.run(&args).status.success());
    }
}
