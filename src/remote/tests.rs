use super::oidc::tests::{WEB_RESOURCE, authorization, authorization_until, web_auth};
use super::*;
use crate::session::SessionMeta;
use crate::store::atomic_write;
use crate::telemetry::SessionLink;
use crate::telemetry::transcript::TranscriptSource;
use crate::test_support::{TempDir, json_field, only_mut, temp_dir};
use axum::body::{Body, BodyDataStream, to_bytes};
use axum::http::request::Builder;
use axum::http::{Request as HttpRequest, header};
use serde_json::{Value, json};
use std::io::Write;
use std::sync::Mutex;
use std::{fs, str};
use tokio::time;
use tokio_stream::StreamExt;
use tower::ServiceExt;

pub(super) struct Fixture {
    pub(super) root: TempDir,
    pub(super) records: Arc<Mutex<Vec<ForegroundSession>>>,
    state: AppState,
}

impl Fixture {
    pub(super) fn new(count: usize) -> Self {
        let root = temp_dir("remote-api");
        atomic_write(
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
            meta: SessionMeta {
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
                session_link: Some(SessionLink::Pi {
                    session_dir: root.to_path_buf(),
                }),
            },
        };
        let records = Arc::new(Mutex::new(vec![record]));
        let source_records = records.clone();
        let state = AppState::new(Arc::new(move |allows| {
            Ok(source_records
                .lock()
                .unwrap()
                .iter()
                .filter(|record| allows(&record.project))
                .cloned()
                .collect())
        }));
        Self {
            root,
            records,
            state,
        }
    }
    fn app(&self) -> Router {
        router(self.state.clone(), web_auth(WEB_RESOURCE))
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

    pub(super) fn append(&self, text: &str) {
        fs::OpenOptions::new()
            .append(true)
            .open(self.root.join("001.jsonl"))
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }
}
pub(super) fn message(text: &str) -> String {
    format!(
        "{}\n",
        json!({"type":"message","message":{"role":"assistant","content":[{"type":"text","text":text}]}})
    )
}
fn request(path: &str) -> HttpRequest<Body> {
    authenticated_request("GET", path, "alice")
        .body(Body::empty())
        .unwrap()
}
pub(super) fn authenticated_request(method: &str, path: &str, subject: &str) -> Builder {
    HttpRequest::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, authorization(subject, WEB_RESOURCE))
}
async fn event(body: &mut BodyDataStream) -> (String, Value) {
    let bytes = time::timeout(Duration::from_secs(3), body.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = str::from_utf8(&bytes).unwrap();
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

const ROUTES: &[&str] = &[
    "/api/telemetry",
    "/api/projects",
    "/api/sessions",
    "/api/sessions/project:session/events",
    "/api/push/vapid",
    "/api/unknown",
];

#[tokio::test]
async fn all_routes_require_a_bearer_for_this_host_and_enforce_project_grants() {
    let fixture = Fixture::new(0);
    {
        let mut records = fixture.records.lock().unwrap();
        let mut other = records.first().unwrap().clone();
        other.project = "other".into();
        records.push(other);
    }
    for path in ROUTES {
        let req = HttpRequest::get(*path)
            .header("Tailscale-User-Login", "alice@example.com")
            .body(Body::empty())
            .unwrap();
        let response = fixture.app().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    let req = HttpRequest::get("/api/sessions")
        .header(
            header::AUTHORIZATION,
            authorization("alice", "https://niles.example.test/h/other/api"),
        )
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        fixture.app().oneshot(req).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    for (method, path, body, expected) in [
        (
            "PUT",
            "/api/projects/project/session",
            "",
            StatusCode::FORBIDDEN,
        ),
        (
            "DELETE",
            "/api/projects/project/session",
            "",
            StatusCode::FORBIDDEN,
        ),
        (
            "DELETE",
            "/api/projects/project/workers/worker",
            "",
            StatusCode::FORBIDDEN,
        ),
        (
            "POST",
            "/api/projects/project/workers/worker/message",
            r#"{"text":"hello"}"#,
            StatusCode::FORBIDDEN,
        ),
        (
            "POST",
            "/api/sessions/project:session/message",
            r#"{"text":"hello"}"#,
            StatusCode::NOT_FOUND,
        ),
    ] {
        let req = authenticated_request(method, path, "bob")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap();
        assert_eq!(
            fixture.app().oneshot(req).await.unwrap().status(),
            expected,
            "{method} {path}"
        );
    }
    for (subject, path, expected) in [
        (
            "bob",
            "/api/sessions/project:session/events",
            StatusCode::OK,
        ),
        (
            "alice",
            "/api/sessions/other:session/events",
            StatusCode::NOT_FOUND,
        ),
        (
            "alice",
            "/api/projects/other/workers/worker/events",
            StatusCode::FORBIDDEN,
        ),
        ("unknown", "/api/sessions", StatusCode::FORBIDDEN),
    ] {
        let req = authenticated_request("GET", path, subject)
            .body(Body::empty())
            .unwrap();
        assert_eq!(fixture.app().oneshot(req).await.unwrap().status(), expected);
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
async fn sse_requires_unexpired_tokens_even_with_jwt_leeway_and_closes_at_expiry() {
    use jsonwebtoken::get_current_timestamp;

    let fixture = Fixture::new(0);
    let request = |expires| {
        HttpRequest::get("/api/sessions/project:session/events")
            .header(
                header::AUTHORIZATION,
                authorization_until("alice", WEB_RESOURCE, expires),
            )
            .body(Body::empty())
            .unwrap()
    };
    let expired = fixture
        .app()
        .oneshot(request(get_current_timestamp().saturating_sub(1)))
        .await
        .unwrap();
    assert_eq!(expired.status(), StatusCode::UNAUTHORIZED);

    let live = fixture
        .app()
        .oneshot(request(get_current_timestamp().saturating_add(2)))
        .await
        .unwrap();
    assert_eq!(live.status(), StatusCode::OK);
    let mut body = live.into_body().into_data_stream();
    assert_eq!(event(&mut body).await.0, "snapshot");
    assert!(
        time::timeout(Duration::from_secs(4), async {
            while body.next().await.is_some() {}
        })
        .await
        .is_ok(),
        "open session stream outlived its token"
    );
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
        assert_eq!(json_field(&snapshot, "/truncated"), true);
        let turns = json_field(&snapshot, "/turns").as_array().unwrap();
        assert_eq!(*turns, (5..205).map(|n| json!({"id":format!("project:session:{n}"),"role":"agent","text":format!("turn {n}")})).collect::<Vec<_>>());
    }
}

#[tokio::test]
async fn sse_follows_partial_appends_status_changes_and_session_exit() {
    let fixture = Fixture::new(0);
    fs::remove_file(fixture.root.join("001.jsonl")).unwrap();
    let mut body = fixture.events().await;
    assert_eq!(
        event(&mut body).await.1,
        json!({"turns":[],"truncated":false})
    );
    assert_eq!(event(&mut body).await.0, "status");
    atomic_write(fixture.root.join("001.jsonl"), message("ready")).unwrap();
    assert_eq!(json_field(&event(&mut body).await.1, "/text"), "ready");
    let line = message("new\ntext");
    fixture.append(&line[..line.len() - 1]);
    only_mut(&mut fixture.records.lock().unwrap()).waiting = false;
    let (kind, status) = event(&mut body).await;
    assert_eq!(kind, "status");
    assert_eq!(json_field(&status, "/waiting"), false);
    fixture.append("\n");
    let (kind, turn) = event(&mut body).await;
    assert_eq!(kind, "turn");
    assert_eq!(
        turn,
        json!({"id":"project:session:1","role":"agent","text":"new\ntext"})
    );
    fixture.records.lock().unwrap().clear();
    assert!(
        time::timeout(Duration::from_secs(3), body.next())
            .await
            .unwrap()
            .is_none()
    );
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
    atomic_write(
        &path,
        format!(
            "{tool}{hidden}{}",
            message("Summary\n\n```rust\nprivate code\n```\n\nDone.")
        ),
    )
    .unwrap();
    let mut body = fixture.events().await;
    let (_, snapshot) = event(&mut body).await;
    assert_eq!(
        snapshot
            .pointer("/turns")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        json_field(&snapshot, "/turns/0/text"),
        "Summary\n\n\n\nDone."
    );
    assert_eq!(event(&mut body).await.0, "status");
    fixture.append(&format!("{tool}{hidden}{}", message("Live prose")));
    let (kind, turn) = event(&mut body).await;
    assert_eq!(kind, "turn");
    assert_eq!(json_field(&turn, "/text"), "Live prose");
}

#[test]
fn worker_feed_streams_turns_without_lead_status_and_ends_when_gone() {
    let fixture = Fixture::new(1);
    let records = fixture.records.clone();
    let lookup: chat::Lookup = Arc::new(move || {
        Ok(records.lock().unwrap().first().map(|record| chat::Chat {
            id: "project/worker".into(),
            transcript: TranscriptSource {
                link: record.meta.session_link.clone(),
                workspace: record.meta.workspace.clone(),
                needle: record.meta.brief.clone(),
                created_at: record.meta.created_at,
            },
            target: None,
            status: None,
        }))
    });
    let source = || lookup().unwrap().unwrap().transcript;
    let mut observer = feed::Follower::default();
    observer.read_new(&source()).unwrap();
    let (mut feed, snapshot) = feed::Feed::open(lookup().unwrap().unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(snapshot)
            .unwrap()
            .pointer("/turns/0/id")
            .unwrap(),
        "project/worker:0"
    );
    assert!(feed.poll(&lookup).unwrap().unwrap().is_empty());
    let replacement = Fixture::new(1);
    *fixture.records.lock().unwrap() = replacement.records.lock().unwrap().clone();
    let events = feed.poll(&lookup).unwrap().unwrap();
    assert_eq!(events.len(), 1);
    assert!(observer.read_new(&source()).unwrap().is_empty());
    fixture.records.lock().unwrap().clear();
    assert!(feed.poll(&lookup).unwrap().is_none());
}

mod collections;
