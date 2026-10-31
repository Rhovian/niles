use super::*;
use crate::config::user;
use axum::{body::Body, extract::ConnectInfo, http::Request};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tower::ServiceExt;

fn config(id: &str, mac: &str) -> BTreeMap<String, FileWakeHost> {
    BTreeMap::from([(
        id.into(),
        FileWakeHost {
            mac: mac.into(),
            broadcast: Ipv4Addr::LOCALHOST,
        },
    )])
}

#[test]
fn wake_configuration_rejects_invalid_ids_and_macs_and_survives_save() {
    for id in ["", "-desk", "../desk", "desk/host", "desk host", "é"] {
        assert!(configured(config(id, "01:23:45:67:89:ab")).is_err(), "{id}");
    }
    for mac in [
        "",
        "1:23:45:67:89:ab",
        "+1:23:45:67:89:ab",
        "gg:23:45:67:89:ab",
        "01:23:45:67:89",
        "01:23:45:67:89:ab:cd",
        "01-23-45-67-89-ab",
    ] {
        assert!(configured(config("desk", mac)).is_err(), "{mac}");
    }
    let mut file = user::FileConfig::default();
    file.remote.wake_hosts = config("desk_1-A", "01:23:45:67:89:AB");
    let path = crate::test_support::temp_test_path("wake-config").join("config.yaml");
    file.save(&path).unwrap();
    let hosts = configured(user::FileConfig::load(&path).unwrap().remote.wake_hosts).unwrap();
    assert_eq!(hosts["desk_1-A"].mac, [1, 0x23, 0x45, 0x67, 0x89, 0xab]);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn wake_sends_exact_magic_packet_and_propagates_socket_errors() {
    let hosts = configured(config("desk", "01:23:45:67:89:ab")).unwrap();
    let receiver = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let SocketAddr::V4(destination) = receiver.local_addr().unwrap() else {
        panic!()
    };
    hosts["desk"].send(destination).unwrap();
    let mut packet = [0; 103];
    assert_eq!(receiver.recv(&mut packet).unwrap(), 102);
    assert_eq!(&packet[..6], &[0xff; 6]);
    for mac in packet[6..102].as_chunks::<6>().0 {
        assert_eq!(mac, &[1, 0x23, 0x45, 0x67, 0x89, 0xab]);
    }
    assert!(
        hosts["desk"]
            .send(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .is_err()
    );
}

#[tokio::test]
async fn wake_endpoint_requires_relay_identity_and_only_wakes_configured_hosts() {
    let mut state = AppState::new(super::super::tests::relay(), Arc::new(|| Ok(Vec::new())));
    state.wake_hosts = Arc::new(configured(config("desk", "01:23:45:67:89:ab")).unwrap());
    for (method, id, login, peer, expected) in [
        ("POST", "desk", true, true, StatusCode::NO_CONTENT),
        ("POST", "unknown", true, true, StatusCode::NOT_FOUND),
        ("POST", "desk", false, true, StatusCode::FORBIDDEN),
        ("POST", "desk", true, false, StatusCode::FORBIDDEN),
        ("GET", "desk", true, true, StatusCode::METHOD_NOT_ALLOWED),
    ] {
        let mut request = Request::builder()
            .method(method)
            .uri(format!("/api/hosts/{id}/wake"));
        if login {
            request = request.header("tailscale-user-login", "alice@example.com");
        }
        if peer {
            request = request.extension(ConnectInfo(
                "100.64.0.1:1234".parse::<SocketAddr>().unwrap(),
            ));
        }
        let response = super::super::router(state.clone())
            .oneshot(
                request
                    .header("content-type", "application/json")
                    .body(Body::from("null"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
}

#[tokio::test]
async fn wake_rejects_simple_posts_and_request_supplied_destinations() {
    let mut state = AppState::new(super::super::tests::relay(), Arc::new(|| Ok(Vec::new())));
    state.wake_hosts = Arc::new(configured(config("desk", "01:23:45:67:89:ab")).unwrap());
    for (content_type, body, expected) in [
        ("text/plain", "null", StatusCode::UNSUPPORTED_MEDIA_TYPE),
        ("application/json", "{}", StatusCode::UNPROCESSABLE_ENTITY),
        (
            "application/json",
            "{\"mac\":\"01:23:45:67:89:ab\"}",
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
    ] {
        let request = Request::post("/api/hosts/desk/wake")
            .header("tailscale-user-login", "alice@example.com")
            .header("content-type", content_type)
            .extension(ConnectInfo(
                "100.64.0.1:1234".parse::<SocketAddr>().unwrap(),
            ))
            .body(Body::from(body))
            .unwrap();
        assert_eq!(
            super::super::router(state.clone())
                .oneshot(request)
                .await
                .unwrap()
                .status(),
            expected
        );
    }
    state.relay.lookup = Arc::new(|_| Ok("other-node".into()));
    let request = Request::post("/api/hosts/desk/wake")
        .header("tailscale-user-login", "alice@example.com")
        .extension(ConnectInfo(
            "100.64.0.1:1234".parse::<SocketAddr>().unwrap(),
        ))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        super::super::router(state)
            .oneshot(request)
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
}
