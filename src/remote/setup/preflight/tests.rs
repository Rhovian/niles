use super::*;

#[test]
fn remote_preflight_compares_release_versions_not_build_hashes() {
    for (json, valid) in [
        (
            r#"{"majorMinorPatch":"1.104.2","daemonLong":"1.104.2-t123"}"#,
            true,
        ),
        (
            r#"{"majorMinorPatch":"1.104.2","daemonLong":"1.96.2-t123"}"#,
            false,
        ),
        (
            r#"{"majorMinorPatch":"1.96.1","daemonLong":"1.96.1"}"#,
            false,
        ),
        (
            r#"{"majorMinorPatch":"1.97.0-dev","daemonLong":"1.97.0-dev"}"#,
            false,
        ),
    ] {
        let version: Version = serde_json::from_str(json).unwrap();
        assert_eq!(version.check().is_ok(), valid, "{json}");
    }
    assert!(serde_json::from_str::<Version>(r#"{"majorMinorPatch":"1.96.2"}"#).is_err());
}

#[test]
fn remote_preflight_requires_certificate_for_this_host() {
    for certs in ["null", "[]", r#"["other.tail.ts.net"]"#] {
        let status: Status = serde_json::from_str(&format!(
            r#"{{"Self":{{"DNSName":"host.tail.ts.net."}},"CertDomains":{certs}}}"#
        ))
        .unwrap();
        assert!(
            status
                .origin()
                .unwrap_err()
                .to_string()
                .contains("https://login.tailscale.com/admin/dns")
        );
    }
    let status: Status = serde_json::from_str(
        r#"{"Self":{"DNSName":"host.tail.ts.net."},"CertDomains":["host.tail.ts.net"]}"#,
    )
    .unwrap();
    assert_eq!(status.origin().unwrap(), "https://host.tail.ts.net");
}

#[test]
fn remote_preflight_launchd_default_does_not_find_homebrew() {
    for json in [
        "{}",
        r#"{"EnvironmentVariables":{}}"#,
        r#"{"EnvironmentVariables":{"PATH":"/usr/bin:/bin"}}"#,
    ] {
        let daemon: Daemon = serde_json::from_str(json).unwrap();
        let error = daemon
            .check(std::path::Path::new("/opt/homebrew/bin/tailscale"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("PlistBuddy"));
        assert!(error.contains("sudo launchctl bootstrap system"));
        assert!(error.contains("Do not use brew services restart"));
    }
    let daemon: Daemon = serde_json::from_str(
        r#"{"EnvironmentVariables":{"PATH":"/opt/homebrew/bin:/usr/bin:/bin"}}"#,
    )
    .unwrap();
    daemon
        .check(std::path::Path::new("/opt/homebrew/bin/tailscale"))
        .unwrap();
}

#[test]
fn remote_preflight_path_resolution_matches_executable_selection() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(first.join("niles"), "not executable").unwrap();
    fs::write(second.join("niles"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(second.join("niles"), fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", first.display(), second.display());
    assert_eq!(resolve("niles", &path).unwrap(), second.join("niles"));
    assert!(resolve("tailscale", &path).is_err());
}
