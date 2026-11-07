use super::*;
use crate::test_support::{TempDir, temp_dir};
use axum::{
    body::{Body, BodyDataStream, to_bytes},
    http::Request as HttpRequest,
};
use serde_json::{Value, json};
use std::{fs, io::Write, sync::Mutex};
use tokio_stream::StreamExt;
use tower::ServiceExt;

struct Fixture {
    root: TempDir,
    records: Arc<Mutex<Vec<ForegroundSession>>>,
    state: AppState,
}

impl Fixture {
    fn new(count: usize) -> Self {
        let root = temp_dir("remote-api");
        fs::write(
            root.join("001.jsonl"),
            (0..count)
                .map(|n| message(&format!("turn {n}")))
                .collect::<String>(),
        )
        .unwrap();
        let record = ForegroundSession {
            project: "project".into(),
            waiting: true,
            worker_count: 2,
            meta: crate::session::SessionMeta {
                setup: None,
                id: "session".into(),
                agent: "pi:model:high".into(),
                agent_family: None,
                entry_skill: "solo".parse().unwrap(),
                model: Some("test-model".into()),
                effort: None,
                created_at: chrono::Utc::now(),
                workspace: root.to_path_buf(),
                brief: root.join("agent.md"),
                lead_pane: None,
                session_link: Some(crate::telemetry::SessionLink::Pi {
                    session_dir: root.to_path_buf(),
                }),
            },
        };
        let records = Arc::new(Mutex::new(vec![record]));
        let source_records = records.clone();
        let state = AppState::new(
            relay(),
            Arc::new(move || Ok(source_records.lock().unwrap().clone())),
        );
        Self {
            root,
            records,
            state,
        }
    }
    fn app(&self) -> Router {
        router(self.state.clone())
    }
    async fn get(&self, path: &str) -> Response {
        self.app().oneshot(request(path)).await.unwrap()
    }

    async fn events(&self) -> BodyDataStream {
        self.get("/api/sessions/project:session/events")
            .await
            .into_body()
            .into_data_stream()
    }

    fn append(&self, text: &str) {
        fs::OpenOptions::new()
            .append(true)
            .open(self.root.join("001.jsonl"))
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }
}
fn message(text: &str) -> String {
    format!(
        "{}\n",
        json!({"type":"message","message":{"role":"assistant","content":[{"type":"text","text":text}]}})
    )
}
pub(super) fn relay() -> auth::Relay {
    auth::Relay {
        node: "relay".into(),
        lookup: Arc::new(|peer| {
            assert_eq!(peer, "100.64.0.1:1234".parse().unwrap());
            Ok("relay".into())
        }),
    }
}
fn request(path: &str) -> HttpRequest<Body> {
    relay_request("GET", path)
        .header("Tailscale-User-Login", "alice@example.com")
        .body(Body::empty())
        .unwrap()
}
pub(super) fn relay_request(method: &str, path: &str) -> axum::http::request::Builder {
    HttpRequest::builder()
        .method(method)
        .uri(path)
        .extension(ConnectInfo(
            "100.64.0.1:1234".parse::<SocketAddr>().unwrap(),
        ))
}
async fn event(body: &mut BodyDataStream) -> (String, Value) {
    let bytes = tokio::time::timeout(Duration::from_secs(3), body.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .unwrap()
            .trim_start()
    };
    (
        field("event:").to_owned(),
        serde_json::from_str(field("data:")).unwrap(),
    )
}

#[test]
fn configuration_requires_relay_node_and_survives_config_save() {
    for node in [
        None,
        Some("".into()),
        Some(" relay".into()),
        Some("relay\n".into()),
    ] {
        assert!(auth::Relay::new(node).is_err());
    }
    let file: user::FileConfig = crate::store::parse_yaml("remote: { relay_node: relay }").unwrap();
    assert_eq!(file.remote.relay_node.as_deref(), Some("relay"));
    let root = temp_dir("remote-config");
    let path = root.join("config.yaml");
    file.save(&path).unwrap();
    assert_eq!(
        user::FileConfig::load(&path)
            .unwrap()
            .remote
            .relay_node
            .as_deref(),
        Some("relay")
    );
}

const ROUTES: &[&str] = &[
    "/api/telemetry",
    "/api/projects",
    "/api/sessions",
    "/api/sessions/project:session/events",
    "/api/push/vapid",
    "/api/unknown",
];

#[tokio::test]
async fn all_routes_require_one_forwarded_identity_and_never_cache() {
    let fixture = Fixture::new(0);
    for path in ROUTES {
        for (login, peer) in [
            (None, true),
            (Some(""), true),
            (Some(" alice"), true),
            (Some("alice,bob"), true),
            (Some("alice@example.com"), false),
        ] {
            let mut req = relay_request("GET", path).body(Body::empty()).unwrap();
            if let Some(login) = login {
                req.headers_mut()
                    .insert("tailscale-user-login", login.parse().unwrap());
            }
            if !peer {
                req.extensions_mut().remove::<ConnectInfo<SocketAddr>>();
            }
            let response = fixture.app().oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        for method in ["GET", "POST", "PUT", "DELETE"] {
            let mut state = fixture.state.clone();
            state.relay.lookup = Arc::new(|_| Ok("other-node".into()));
            let mut req = request(path);
            *req.method_mut() = method.parse().unwrap();
            req.headers_mut()
                .insert("x-forwarded-for", "100.64.0.1".parse().unwrap());
            let response = router(state).oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
    }
    let mut duplicate = request("/api/sessions");
    duplicate
        .headers_mut()
        .append("tailscale-user-login", "alice@example.com".parse().unwrap());
    let mut invalid = request("/api/sessions");
    invalid.headers_mut().insert(
        "tailscale-user-login",
        header::HeaderValue::from_bytes(&[0xff]).unwrap(),
    );
    for req in [duplicate, invalid] {
        let response = fixture.app().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    for login in ["alice@example.com", "other@example.net"] {
        let mut req = request("/api/sessions");
        req.headers_mut()
            .insert("tailscale-user-login", login.parse().unwrap());
        assert_eq!(
            fixture.app().oneshot(req).await.unwrap().status(),
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn sessions_match_v1_and_resolve_only_enumerated_live_ids() {
    let fixture = Fixture::new(0);
    let response = fixture.get("/api/sessions").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert!(
        !response
            .headers()
            .contains_key("access-control-allow-origin")
    );
    let json: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(
        json,
        json!({"api_version":1,"sessions":[{"id":"project:session","project":"project",
        "agent":{"family":"pi","model":"test-model","entry_skill":"solo"},"waiting":true,"worker_count":2}]})
    );
    for path in [
        "/api/sessions/unknown/events",
        "/api/sessions/%2Fetc%2Fpasswd/events",
        "/",
    ] {
        let response = fixture.get(path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
}

#[tokio::test]
async fn sse_snapshots_are_bounded_ordered_and_repeat_on_reconnect_without_replay() {
    let fixture = Fixture::new(205);
    for last_id in [None, Some("obsolete-cursor")] {
        let mut req = request("/api/sessions/project%3Asession/events");
        if let Some(id) = last_id {
            req.headers_mut()
                .insert("last-event-id", id.parse().unwrap());
        }
        let response = fixture.app().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/event-stream"
        );
        let (kind, snapshot) = event(&mut response.into_body().into_data_stream()).await;
        assert_eq!(kind, "snapshot");
        assert_eq!(snapshot["truncated"], true);
        let turns = snapshot["turns"].as_array().unwrap();
        assert_eq!(turns.len(), 200);
        assert_eq!(
            turns[0],
            json!({"id":"project:session:5","role":"agent","text":"turn 5"})
        );
        assert_eq!(turns[199]["text"], "turn 204");
    }
}

#[tokio::test]
async fn sse_follows_partial_appends_status_changes_and_session_exit() {
    let fixture = Fixture::new(205);
    let mut body = fixture.events().await;
    let (_, snapshot) = event(&mut body).await;
    assert_eq!(snapshot["truncated"], true);
    assert_eq!(event(&mut body).await.0, "status");
    let line = message("new\ntext");
    fixture.append(&line[..line.len() - 1]);
    fixture.records.lock().unwrap()[0].waiting = false;
    let (kind, status) = event(&mut body).await;
    assert_eq!(kind, "status");
    assert_eq!(status["waiting"], false);
    fixture.append("\n");
    let (kind, turn) = event(&mut body).await;
    assert_eq!(kind, "turn");
    assert_eq!(
        turn,
        json!({"id":"project:session:205","role":"agent","text":"new\ntext"})
    );
    fixture.records.lock().unwrap().clear();
    assert!(
        tokio::time::timeout(Duration::from_secs(3), body.next())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn unavailable_transcripts_can_appear_after_the_initial_snapshot() {
    let fixture = Fixture::new(0);
    fs::remove_file(fixture.root.join("001.jsonl")).unwrap();
    let mut body = fixture.events().await;
    assert_eq!(
        event(&mut body).await.1,
        json!({"turns":[],"truncated":false})
    );
    assert_eq!(event(&mut body).await.0, "status");
    fs::write(fixture.root.join("001.jsonl"), message("ready")).unwrap();
    assert_eq!(event(&mut body).await.1["text"], "ready");
}

#[test]
fn mobile_text_omits_tools_and_assistant_code_blocks() {
    use crate::telemetry::transcript::{TextBlock, TextRole, TextTurn};
    let mobile = |role, text: &str| {
        super::feed::mobile_text(TextTurn {
            role,
            timestamp: None,
            content: TextBlock::Text { text: text.into() },
        })
    };
    for code in [
        "```diff\n-secret\n```",
        "~~~~rust\nsecret\n~~~~",
        "    secret\n",
        "```\nsecret",
        "> ```\n> secret\n> ```",
        "- item\n\n    ```\n    secret\n    ```",
    ] {
        let text = format!("Before 🦀\n\n{code}\n\nAfter `inline`.");
        let visible = mobile(TextRole::Assistant, &text).unwrap();
        assert!(!visible.contains("secret"), "{visible}");
        assert!(visible.contains("Before 🦀"));
        // An unclosed fence consumes the remainder according to Markdown's grammar.
        assert_eq!(
            visible.contains("After `inline`."),
            !code.ends_with("secret")
        );
        assert_eq!(mobile(TextRole::User, &text), Some(text));
    }
    assert_eq!(
        mobile(TextRole::Assistant, "```rust\nfn main() {}\n```"),
        None
    );
}

#[tokio::test]
async fn mobile_sse_filters_snapshot_and_live_turns_before_delivery() {
    let fixture = Fixture::new(0);
    let path = fixture.root.join("001.jsonl");
    let tool = format!(
        "{}\n",
        json!({"type":"message","message":{"role":"assistant",
        "content":[{"type":"toolCall","name":"Bash","arguments":{"command":"private command"}}]}})
    );
    let hidden = message("```diff\nprivate diff\n```");
    fs::write(
        &path,
        format!(
            "{tool}{hidden}{}",
            message("Summary\n\n```rust\nprivate code\n```\n\nDone.")
        ),
    )
    .unwrap();
    let mut body = fixture.events().await;
    let (_, snapshot) = event(&mut body).await;
    assert_eq!(snapshot["turns"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["turns"][0]["text"], "Summary\n\n\n\nDone.");
    assert_eq!(event(&mut body).await.0, "status");
    fixture.append(&format!("{tool}{hidden}{}", message("Live prose")));
    let (kind, turn) = event(&mut body).await;
    assert_eq!(kind, "turn");
    assert_eq!(turn["text"], "Live prose");
}

#[test]
fn worker_feed_streams_turns_without_lead_status_and_ends_when_gone() {
    let fixture = Fixture::new(1);
    let records = fixture.records.clone();
    let lookup: chat::Lookup = Arc::new(move || {
        Ok(records.lock().unwrap().first().map(|record| chat::Chat {
            id: "project/worker".into(),
            transcript: crate::telemetry::transcript::TranscriptSource {
                link: record.meta.session_link.clone(),
                workspace: record.meta.workspace.clone(),
                needle: record.meta.brief.clone(),
                created_at: record.meta.created_at,
            },
            target: None,
            status: None,
        }))
    });
    let (mut feed, snapshot) = feed::Feed::open(lookup().unwrap().unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(snapshot).unwrap()["turns"][0]["id"],
        "project/worker:0"
    );
    assert!(feed.poll(&lookup).unwrap().unwrap().is_empty());
    fixture.append(&message("worker reply"));
    let events = feed.poll(&lookup).unwrap().unwrap();
    assert_eq!(events.len(), 1);
    fixture.records.lock().unwrap().clear();
    assert!(feed.poll(&lookup).unwrap().is_none());
}

mod collections;
