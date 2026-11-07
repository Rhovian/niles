use super::*;
use crate::{
    remote::tests::relay_request,
    test_support::{TempDir, assert_err_contains, temp_dir},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use std::{fs, os::unix::fs::PermissionsExt};
use web_push::SubscriptionInfo;

fn keyed_subscription(endpoint: &str) -> (Subscription, ece::EcKeyComponents, [u8; 16]) {
    let (key, auth) = ece::generate_keypair_and_auth_secret().unwrap();
    let info = SubscriptionInfo::new(
        endpoint.to_owned(),
        URL_SAFE_NO_PAD.encode(key.pub_as_raw().unwrap()),
        URL_SAFE_NO_PAD.encode(auth),
    );
    (
        Subscription::try_from(info).unwrap(),
        key.raw_components().unwrap(),
        auth,
    )
}

fn fixture() -> (TempDir, Store, Subscription) {
    let root = temp_dir("remote-push");
    let store = Store::create(root.join("remote"), "mailto:operator@example.com").unwrap();
    let (subscription, _, _) = keyed_subscription("https://fcm.googleapis.com/fcm/send/device");
    (root, store, subscription)
}

fn session(id: &str, family: &str) -> Session {
    Session {
        id: id.into(),
        project: "p".into(),
        agent: super::super::Agent {
            model: None,
            family: family.into(),
            entry_skill: "solo".into(),
        },
        waiting: true,
        worker_count: 0,
    }
}
#[test]
fn private_keys_and_device_files_survive_restart_without_rotation() {
    let (root, store, subscription) = fixture();
    let key = store.public_key().unwrap();
    store
        .save("alice".into(), subscription.clone(), None)
        .unwrap();
    store
        .save("alice".into(), subscription.clone(), None)
        .unwrap();
    assert_eq!(store.subscriptions().unwrap().len(), 1);
    assert_eq!(
        fs::metadata(root.join("remote/vapid.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    for file in fs::read_dir(root.join("remote/subscriptions")).unwrap() {
        assert_eq!(
            file.unwrap().metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let path = root.join("remote");
    assert!(Store::create(path.clone(), "mailto:operator@example.com").is_err());
    let loaded = Store::load(path).unwrap();
    assert_eq!(loaded.public_key().unwrap(), key);
    assert_eq!(
        loaded.subscriptions().unwrap()[0].subscription.0,
        subscription.0
    );
    let (subscription, receiver, auth) = keyed_subscription(&subscription.0.endpoint);
    let plaintext = br#"{"session_id":"s","project":"p","reason":"waiting"}"#;
    let payload = loaded
        .message(&subscription, plaintext)
        .unwrap()
        .payload
        .unwrap();
    assert_eq!(
        ece::decrypt(&receiver, &auth, &payload.content).unwrap(),
        plaintext
    );
    assert!(!payload.crypto_headers.is_empty());
}

#[test]
fn rejects_non_browser_endpoints_and_bad_encryption_keys() {
    let (_root, store, subscription) = fixture();
    for endpoint in [
        "http://fcm.googleapis.com/device",
        "https://127.0.0.1/api",
        "https://fcm.googleapis.com.evil.example/a",
        "https://web.push.apple.com:8443/a",
        "https://user@fcm.googleapis.com/a",
    ] {
        let mut info = subscription.0.clone();
        info.endpoint = endpoint.into();
        assert!(Subscription::try_from(info).is_err());
    }
    let mut info = subscription.0.clone();
    info.keys.auth = "invalid".into();
    assert!(Subscription::try_from(info).is_err());
    assert_eq!(store.subscriptions().unwrap().len(), 0);
}
#[test]
fn waiting_transitions_include_solo_without_repeating_or_retaining_exited_sessions() {
    let session = session("s", "claude");
    let mut previous = HashSet::new();
    assert_eq!(
        newly_waiting(&mut previous, vec![session.clone()]),
        vec![session.clone()]
    );
    assert!(newly_waiting(&mut previous, vec![session.clone()]).is_empty());
    let mut working = session.clone();
    working.waiting = false;
    assert!(newly_waiting(&mut previous, vec![working]).is_empty());
    assert_eq!(newly_waiting(&mut previous, vec![session.clone()]).len(), 1);
    newly_waiting(&mut previous, vec![]);
    assert_eq!(newly_waiting(&mut previous, vec![session]).len(), 1);
}
#[test]
fn prunes_only_gone_endpoints_and_propagates_other_failures() {
    let (_root, store, subscription) = fixture();
    for code in [404, 410] {
        store
            .save("alice".into(), subscription.clone(), None)
            .unwrap();
        let info = serde_json::from_value(
            serde_json::json!({"code":code,"errno":0,"error":"gone","message":"gone"}),
        )
        .unwrap();
        let error = if code == 404 {
            WebPushError::EndpointNotFound(info)
        } else {
            WebPushError::EndpointNotValid(info)
        };
        finish_delivery(&store, &subscription, Err(error)).unwrap();
        assert!(store.subscriptions().unwrap().is_empty());
    }
    store
        .save("alice".into(), subscription.clone(), None)
        .unwrap();
    assert!(finish_delivery(&store, &subscription, Err(WebPushError::InvalidResponse)).is_err());
    assert_eq!(store.subscriptions().unwrap().len(), 1);
}
#[tokio::test]
async fn push_routes_authorize_validate_and_never_cache() {
    use axum::body::{Body, to_bytes};
    use tower::ServiceExt;
    let (_root, store, subscription) = fixture();
    let mut state = AppState::new(super::super::tests::relay(), Arc::new(|| Ok(vec![])));
    state.push = Some(Arc::new(store));
    let app = super::super::router(state);
    let subscription = serde_json::to_string(&subscription).unwrap();
    for (method, path, login, body, expected) in [
        ("GET", "/api/push/vapid", "", "", 403),
        ("GET", "/api/push/vapid", "alice", "", 200),
        (
            "POST",
            "/api/push/subscribe",
            "alice",
            subscription.as_str(),
            200,
        ),
        ("POST", "/api/push/subscribe", "alice", "{}", 422),
    ] {
        let request = relay_request(method, path)
            .header("Tailscale-User-Login", login)
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(
            response.status().as_u16(),
            expected,
            "{method} {path} {login}"
        );
        assert_eq!(response.headers()["cache-control"], "no-store");
        if method == "GET" && expected == 200 {
            let value: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 1024).await.unwrap())
                    .unwrap();
            assert_eq!(
                URL_SAFE_NO_PAD
                    .decode(value["public_key"].as_str().unwrap())
                    .unwrap()
                    .len(),
                65
            );
        }
    }
}

#[tokio::test]
async fn polling_failure_is_reported_without_any_browser_connection() {
    let (_root, store, _) = fixture();
    let source: Source = Arc::new(|| anyhow::bail!("catalog failure"));
    assert_err_contains(
        tokio::time::timeout(Duration::from_secs(3), watch(source, Arc::new(store)))
            .await
            .unwrap(),
        "catalog failure",
    );
}

#[test]
fn shared_key_and_subscription_deliver_host_qualified_payloads_after_restart() {
    let (root, first, subscription) = fixture();
    let second_root = root.join("second");
    fs::create_dir(&second_root).unwrap();
    fs::set_permissions(&second_root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::copy(
        root.join("remote/vapid.json"),
        second_root.join("vapid.json"),
    )
    .unwrap();
    let second = Store::load(second_root.clone()).unwrap();
    assert_eq!(first.public_key().unwrap(), second.public_key().unwrap());
    let session = session("same-session", "codex");
    for (store, id) in [(&first, "desk"), (&second, "laptop")] {
        let registration: store::Registration = serde_json::from_value(serde_json::json!({
            "endpoint": subscription.0.endpoint, "keys": subscription.0.keys,
            "host": {"id": id, "name": id},
        }))
        .unwrap();
        store
            .save("alice".into(), registration.subscription, registration.host)
            .unwrap();
        let devices = store.subscriptions().unwrap();
        let payload: serde_json::Value =
            serde_json::from_slice(&devices[0].payload(&session).unwrap()).unwrap();
        assert_eq!(payload["host"]["id"], id);
        assert_eq!(payload["session_id"], "same-session");
        store
            .message(
                &devices[0].subscription,
                &serde_json::to_vec(&payload).unwrap(),
            )
            .unwrap();
    }
    let loaded = Store::load(second_root).unwrap();
    assert_eq!(
        loaded.subscriptions().unwrap()[0].host.as_ref().unwrap().id,
        "laptop"
    );
}
