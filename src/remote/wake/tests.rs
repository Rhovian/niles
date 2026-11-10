use super::*;
use crate::config::user;
use crate::remote::tests::relay_request;
use crate::test_support::temp_dir;
use axum::body::Body;
use axum::extract::ConnectInfo;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
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
    let root = temp_dir("wake-config");
    let path = root.join("config.yaml");
    file.save(&path).unwrap();
    let hosts = configured(user::FileConfig::load(&path).unwrap().remote.wake_hosts).unwrap();
    assert_eq!(
        hosts.get("desk_1-A").unwrap().mac,
        [1, 0x23, 0x45, 0x67, 0x89, 0xab]
    );
}

#[test]
fn wake_sends_exact_magic_packet_and_propagates_socket_errors() {
    let hosts = configured(config("desk", "01:23:45:67:89:ab")).unwrap();
    let receiver = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let destination = SocketAddrV4::new(Ipv4Addr::LOCALHOST, receiver.local_addr().unwrap().port());
    hosts.get("desk").unwrap().send(destination).unwrap();
    let mut packet = [0; 103];
    assert_eq!(receiver.recv(&mut packet).unwrap(), 102);
    assert_eq!(&packet[..6], &[0xff; 6]);
    for mac in packet[6..102].as_chunks::<6>().0 {
        assert_eq!(mac, &[1, 0x23, 0x45, 0x67, 0x89, 0xab]);
    }
    assert!(
        hosts
            .get("desk")
            .unwrap()
            .send(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .is_err()
    );
}

#[tokio::test]
async fn wake_routes_require_relay_identity_and_only_accept_configured_destinations() {
    let mut state = AppState::new(super::super::tests::relay(), Arc::new(|| Ok(Vec::new())));
    state.wake_hosts = Arc::new(configured(config("desk", "01:23:45:67:89:ab")).unwrap());
    #[derive(Debug)]
    enum Identity {
        Relay,
        NoLogin,
        NoPeer,
        OtherNode,
    }
    use Identity::{NoLogin, NoPeer, OtherNode, Relay};
    for (method, id, identity, content_type, body, expected) in [
        ("POST", "desk", Relay, "application/json", "null", 204),
        ("POST", "unknown", Relay, "application/json", "null", 404),
        ("POST", "desk", NoLogin, "application/json", "null", 403),
        ("POST", "desk", NoPeer, "application/json", "null", 403),
        ("GET", "desk", Relay, "application/json", "null", 405),
        ("POST", "desk", Relay, "text/plain", "null", 415),
        ("POST", "desk", Relay, "application/json", "{}", 422),
        (
            "POST",
            "desk",
            Relay,
            "application/json",
            "{\"mac\":\"01:23:45:67:89:ab\"}",
            413,
        ),
        ("POST", "desk", OtherNode, "", "", 403),
    ] {
        let mut state = state.clone();
        let mut request = relay_request(method, &format!("/api/hosts/{id}/wake"))
            .header("tailscale-user-login", "alice@example.com")
            .header("content-type", content_type)
            .body(Body::from(body))
            .unwrap();
        match identity {
            NoLogin => {
                request.headers_mut().remove("tailscale-user-login");
            }
            NoPeer => {
                request.extensions_mut().remove::<ConnectInfo<SocketAddr>>();
            }
            OtherNode => state.relay.lookup = Arc::new(|_| Ok("other-node".into())),
            Relay => {}
        }
        let response = super::super::router(state).oneshot(request).await.unwrap();
        assert_eq!(
            response.status().as_u16(),
            expected,
            "{method} {id} {identity:?} {body}"
        );
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
}
