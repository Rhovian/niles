use super::*;
use axum::{
    body::{Body, BodyDataStream, to_bytes},
    http::Request as HttpRequest,
};
use camino::Utf8PathBuf;
use serde_json::{Value, json};
use std::{fs, io::Write, sync::Mutex};
use tokio_stream::StreamExt;
use tower::ServiceExt;

struct Fixture {
    root: Utf8PathBuf,
    records: Arc<Mutex<Vec<ForegroundSession>>>,
    state: AppState,
}

impl Fixture {
    fn new(count: usize) -> Self {
        let root = crate::test_support::temp_test_path("remote-api");
        fs::create_dir_all(&root).unwrap();
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
                id: "session".into(),
                agent: "pi:model:high".into(),
                agent_family: None,
                entry_skill: "solo".into(),
                model: Some("test-model".into()),
                effort: None,
                created_at: chrono::Utc::now(),
                workspace: root.clone(),
                brief: root.join("agent.md"),
                lead_pane: None,
                session_link: Some(crate::telemetry::SessionLink::Pi {
                    session_dir: root.clone(),
                }),
            },
        };
        let records = Arc::new(Mutex::new(vec![record]));
        let source_records = records.clone();
        let state = AppState::new(
            vec!["alice@example.com".into()],
            Arc::new(move || Ok(source_records.lock().unwrap().clone())),
        )
        .unwrap();
        Self {
            root,
            records,
            state,
        }
    }
    fn app(&self) -> Router {
        router(self.state.clone())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn message(text: &str) -> String {
    format!(
        "{}\n",
        json!({"type":"message","message":{"role":"assistant","content":[{"type":"text","text":text}]}})
    )
}
fn request(path: &str) -> HttpRequest<Body> {
    HttpRequest::builder()
        .uri(path)
        .header("Tailscale-User-Login", "alice@example.com")
        .body(Body::empty())
        .unwrap()
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
fn configuration_requires_explicit_nonempty_users_and_survives_config_save() {
    for users in [
        vec![],
        vec!["".into()],
        vec![" alice".into()],
        vec!["alice\n".into()],
    ] {
        assert!(AppState::new(users, Arc::new(|| Ok(Vec::new()))).is_err());
    }
    let file: user::FileConfig =
        crate::store::parse_yaml("remote: { allowed_users: ['alice@example.com'] }").unwrap();
    assert_eq!(file.remote.allowed_users, ["alice@example.com"]);
    let path = crate::test_support::temp_test_path("remote-config").join("config.yaml");
    file.save(&path).unwrap();
    assert_eq!(
        user::FileConfig::load(&path).unwrap().remote.allowed_users,
        ["alice@example.com"]
    );
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn all_routes_require_one_allowed_identity_and_never_cache() {
    let fixture = Fixture::new(0);
    for path in [
        "/api/projects",
        "/api/sessions",
        "/api/sessions/project:session/events",
        "/api/unknown",
    ] {
        for login in [None, Some("mallory@example.com")] {
            let mut req = HttpRequest::builder()
                .uri(path)
                .body(Body::empty())
                .unwrap();
            if let Some(login) = login {
                req.headers_mut()
                    .insert("tailscale-user-login", login.parse().unwrap());
            }
            let response = fixture.app().oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
    }
    let mut duplicate = request("/api/sessions");
    duplicate
        .headers_mut()
        .append("tailscale-user-login", "alice@example.com".parse().unwrap());
    assert_eq!(
        fixture.app().oneshot(duplicate).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let mut invalid = request("/api/sessions");
    invalid.headers_mut().insert(
        "tailscale-user-login",
        header::HeaderValue::from_bytes(&[0xff]).unwrap(),
    );
    assert_eq!(
        fixture.app().oneshot(invalid).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn sessions_match_v1_and_resolve_only_enumerated_live_ids() {
    let fixture = Fixture::new(0);
    let response = fixture
        .app()
        .oneshot(request("/api/sessions"))
        .await
        .unwrap();
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
        let response = fixture.app().oneshot(request(path)).await.unwrap();
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
    let response = fixture
        .app()
        .oneshot(request("/api/sessions/project:session/events"))
        .await
        .unwrap();
    let mut body = response.into_body().into_data_stream();
    let (_, snapshot) = event(&mut body).await;
    assert_eq!(snapshot["truncated"], true);
    assert_eq!(event(&mut body).await.0, "status");
    let mut append = fs::OpenOptions::new()
        .append(true)
        .open(fixture.root.join("001.jsonl"))
        .unwrap();
    let line = message("new\ntext");
    append
        .write_all(&line.as_bytes()[..line.len() - 1])
        .unwrap();
    fixture.records.lock().unwrap()[0].waiting = false;
    let (kind, status) = event(&mut body).await;
    assert_eq!(kind, "status");
    assert_eq!(status["waiting"], false);
    append.write_all(b"\n").unwrap();
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
    let response = fixture
        .app()
        .oneshot(request("/api/sessions/project:session/events"))
        .await
        .unwrap();
    let mut body = response.into_body().into_data_stream();
    assert_eq!(
        event(&mut body).await.1,
        json!({"turns":[],"truncated":false})
    );
    assert_eq!(event(&mut body).await.0, "status");
    fs::write(fixture.root.join("001.jsonl"), message("ready")).unwrap();
    assert_eq!(event(&mut body).await.1["text"], "ready");
}

#[tokio::test]
async fn internal_errors_are_not_returned_to_the_client_or_cached() {
    let state = AppState::new(
        vec!["alice@example.com".into()],
        Arc::new(|| anyhow::bail!("private path /secret")),
    )
    .unwrap();
    let response = router(state)
        .oneshot(request("/api/sessions"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert!(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .is_empty()
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
    fs::write(
        &path,
        format!(
            "{tool}{hidden}{}",
            message("Summary\n\n```rust\nprivate code\n```\n\nDone.")
        ),
    )
    .unwrap();
    let response = fixture
        .app()
        .oneshot(request("/api/sessions/project:session/events"))
        .await
        .unwrap();
    let mut body = response.into_body().into_data_stream();
    let (_, snapshot) = event(&mut body).await;
    assert_eq!(snapshot["turns"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["turns"][0]["text"], "Summary\n\n\n\nDone.");
    assert_eq!(event(&mut body).await.0, "status");
    fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(format!("{tool}{hidden}{}", message("Live prose")).as_bytes())
        .unwrap();
    let (kind, turn) = event(&mut body).await;
    assert_eq!(kind, "turn");
    assert_eq!(turn["text"], "Live prose");
}

#[tokio::test]
async fn projects_are_versioned_uncached_and_collection_errors_are_private() {
    let mut fixture = Fixture::new(0);
    fixture.state.projects = Arc::new(|| Ok(Vec::new()));
    let response = fixture
        .app()
        .oneshot(request("/api/projects"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap(),
        json!({"api_version": 1, "projects": []})
    );
    fixture.state.projects = Arc::new(|| anyhow::bail!("private registry path"));
    let response = fixture
        .app()
        .oneshot(request("/api/projects"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert!(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .is_empty()
    );
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
    fs::OpenOptions::new()
        .append(true)
        .open(fixture.root.join("001.jsonl"))
        .unwrap()
        .write_all(message("worker reply").as_bytes())
        .unwrap();
    let events = feed.poll(&lookup).unwrap().unwrap();
    assert_eq!(events.len(), 1);
    fixture.records.lock().unwrap().clear();
    assert!(feed.poll(&lookup).unwrap().is_none());
}
