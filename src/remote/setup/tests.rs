use super::*;

#[test]
fn remote_setup_preserves_other_mounts_and_known_metadata() {
    for json in ["{}", include_str!("serve-status.json")] {
        serde_json::from_str::<Serve>(json)
            .unwrap()
            .check()
            .unwrap();
    }
    for path in ["/", "/api", "/api/", "/api/sessions"] {
        let json = serde_json::json!({"Web":{"host:443":{"Handlers":{path:{"Proxy":"http://localhost"}}}}});
        assert!(
            serde_json::from_value::<Serve>(json)
                .unwrap()
                .check()
                .is_err(),
            "{path}"
        );
    }
}

#[test]
fn remote_setup_rejects_unknown_serve_schema() {
    for field in ["WebV2", "TCPV2", "NewSetting"] {
        let json = serde_json::json!({"TCP":{}, field:{}});
        assert!(
            serde_json::from_value::<Serve>(json)
                .unwrap()
                .check()
                .is_err()
        );
    }
}

#[test]
fn remote_setup_requires_supported_numeric_tailscale_version() {
    for version in ["1.96.2\n  commit: x", "1.100.0", "2.0.0"] {
        check_version(version).unwrap();
    }
    for version in [
        "",
        "garbage",
        "1.96.1",
        "1.96",
        "1.96.2.0",
        "1.97.0-dev",
        "1.96.2-t123",
    ] {
        assert!(check_version(version).is_err(), "{version}");
    }
}

#[test]
fn remote_setup_verification_requires_the_authorized_api_contract() {
    let body = r#"{"api_version":1,"sessions":[]}"#;
    verify(&format!("{body}\n200\nno-store")).unwrap();
    for output in [
        format!("{body}\n403\nno-store"),
        format!("{body}\n200\n"),
        "<html>shell</html>\n200\nno-store".into(),
        "{\"api_version\":2,\"sessions\":[]}\n200\nno-store".into(),
        "{\"api_version\":1}\n200\nno-store".into(),
        "missing footer".into(),
    ] {
        assert!(verify(&output).is_err(), "{output}");
    }
}

#[test]
fn remote_setup_service_preserves_literal_paths_in_both_formats() {
    use service::{Manager, Service};
    let home = Utf8Path::new("/home/a & b");
    let executable = Utf8Path::new("/bin/niles %$\"&");
    let service =
        Service::render(Manager::Launchd, home, executable, "/bin:/my & tools", 9123).unwrap();
    assert_eq!(
        service.path,
        home.join("Library/LaunchAgents/dev.niles.remote.plist")
    );
    assert!(service.contents.contains("/bin/niles %$&quot;&amp;"));
    assert!(service.contents.contains("<string>9123</string>"));
    #[cfg(target_os = "macos")]
    {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &service.contents).unwrap();
        let output = std::process::Command::new("/usr/bin/plutil")
            .arg("-lint")
            .arg(file.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    let service =
        Service::render(Manager::Systemd, home, executable, "/bin:/my tools", 9123).unwrap();
    assert_eq!(
        service.path,
        home.join(".config/systemd/user/niles-remote.service")
    );
    assert!(
        service
            .contents
            .contains(r#"ExecStart="/bin/niles %%$$\"&" remote --port 9123"#)
    );
    assert!(service.contents.contains("Restart=always"));
    assert!(
        Service::render(
            Manager::Systemd,
            home,
            Utf8Path::new("/bin/bad\npath"),
            "/bin",
            9123
        )
        .is_err()
    );
}
