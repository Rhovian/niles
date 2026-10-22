use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use std::{fs, os::unix::fs::PermissionsExt};
use web_push::SubscriptionInfo;

fn fixture() -> (tempfile::TempDir, Store, Subscription) {
    let root = tempfile::tempdir().unwrap();
    let store = Store::create(
        camino::Utf8PathBuf::from_path_buf(root.path().join("remote")).unwrap(),
        "mailto:operator@example.com",
    )
    .unwrap();
    let (key, auth) = ece::generate_keypair_and_auth_secret().unwrap();
    let info = SubscriptionInfo::new(
        "https://fcm.googleapis.com/fcm/send/device".to_owned(),
        URL_SAFE_NO_PAD.encode(key.pub_as_raw().unwrap()),
        URL_SAFE_NO_PAD.encode(auth),
    );
    (root, store, Subscription::try_from(info).unwrap())
}
#[test]
fn private_keys_and_device_files_survive_restart_without_rotation() {
    let (root, store, subscription) = fixture();
    let key = store.public_key().unwrap();
    store.save("alice".into(), subscription.clone()).unwrap();
    store.save("alice".into(), subscription.clone()).unwrap();
    assert_eq!(store.subscriptions(&["alice".into()]).unwrap().len(), 1);
    assert_eq!(
        fs::metadata(root.path().join("remote/vapid.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    for file in fs::read_dir(root.path().join("remote/subscriptions")).unwrap() {
        assert_eq!(
            file.unwrap().metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let path = camino::Utf8PathBuf::from_path_buf(root.path().join("remote")).unwrap();
    assert!(Store::create(path.clone(), "mailto:operator@example.com").is_err());
    let loaded = Store::load(path).unwrap();
    assert!(
        loaded
            .subscriptions(&["revoked".into()])
            .unwrap()
            .is_empty()
    );
    assert_eq!(loaded.public_key().unwrap(), key);
    assert_eq!(
        loaded.subscriptions(&["alice".into()]).unwrap()[0].0,
        subscription.0
    );
    let (receiver, auth) = ece::generate_keypair_and_auth_secret().unwrap();
    let info = SubscriptionInfo::new(
        subscription.0.endpoint,
        URL_SAFE_NO_PAD.encode(receiver.pub_as_raw().unwrap()),
        URL_SAFE_NO_PAD.encode(auth),
    );
    let subscription = Subscription::try_from(info).unwrap();
    let plaintext = br#"{"session_id":"s","project":"p","reason":"waiting"}"#;
    let payload = loaded
        .message(&subscription, plaintext)
        .unwrap()
        .payload
        .unwrap();
    assert_eq!(
        ece::decrypt(&receiver.raw_components().unwrap(), &auth, &payload.content).unwrap(),
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
    assert_eq!(store.subscriptions(&["alice".into()]).unwrap().len(), 0);
}
#[test]
fn waiting_transitions_include_solo_without_repeating_or_retaining_exited_sessions() {
    let session = Session {
        id: "s".into(),
        project: "p".into(),
        agent: super::super::Agent {
            model: None,
            family: "claude".into(),
            entry_skill: "solo".into(),
        },
        waiting: true,
        worker_count: 0,
    };
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
        store.save("alice".into(), subscription.clone()).unwrap();
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
        assert!(store.subscriptions(&["alice".into()]).unwrap().is_empty());
    }
    store.save("alice".into(), subscription.clone()).unwrap();
    assert!(finish_delivery(&store, &subscription, Err(WebPushError::InvalidResponse)).is_err());
    assert_eq!(store.subscriptions(&["alice".into()]).unwrap().len(), 1);
}
#[tokio::test]
async fn push_routes_authorize_validate_and_never_cache() {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let (_root, store, subscription) = fixture();
    let mut state = AppState::new(vec!["alice".into()], Arc::new(|| Ok(vec![]))).unwrap();
    state.push = Some(Arc::new(store));
    let app = super::super::router(state);
    let request = |method, path, login, body: String| {
        Request::builder()
            .method(method)
            .uri(path)
            .header("Tailscale-User-Login", login)
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap()
    };
    let forbidden = app
        .clone()
        .oneshot(request("GET", "/api/push/vapid", "mallory", String::new()))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    let key = app
        .clone()
        .oneshot(request("GET", "/api/push/vapid", "alice", String::new()))
        .await
        .unwrap();
    assert_eq!(key.status(), StatusCode::OK);
    assert_eq!(key.headers()["cache-control"], "no-store");
    let value: serde_json::Value =
        serde_json::from_slice(&to_bytes(key.into_body(), 1024).await.unwrap()).unwrap();
    assert_eq!(
        URL_SAFE_NO_PAD
            .decode(value["public_key"].as_str().unwrap())
            .unwrap()
            .len(),
        65
    );
    let saved = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/push/subscribe",
            "alice",
            serde_json::to_string(&subscription).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(saved.status(), StatusCode::OK);
    assert_eq!(saved.headers()["cache-control"], "no-store");
    let invalid = app
        .oneshot(request("POST", "/api/push/subscribe", "alice", "{}".into()))
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn polling_failure_is_reported_without_any_browser_connection() {
    let (_root, store, _) = fixture();
    let source: Source = Arc::new(|| anyhow::bail!("catalog failure"));
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        watch(source, Arc::new(store), Arc::new(vec!["alice".into()])),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.to_string().contains("catalog failure"));
}
