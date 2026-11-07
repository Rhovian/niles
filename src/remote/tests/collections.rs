use super::*;

fn collection_source<T>(failing: bool) -> impl Fn() -> Result<Vec<T>> {
    move || {
        anyhow::ensure!(!failing, "private collection path");
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn versioned_uncached_and_errors_are_private() {
    type SourceSetter = fn(&mut AppState, bool);
    let routes: [(&str, &str, SourceSetter); 3] = [
        ("/api/projects", "projects", |state, failing| {
            state.projects = Arc::new(collection_source(failing));
        }),
        ("/api/telemetry", "sessions", |state, failing| {
            state.telemetry = Arc::new(collection_source(failing));
        }),
        ("/api/sessions", "sessions", |state, failing| {
            state.source = Arc::new(collection_source(failing));
        }),
    ];
    for (path, field, set_source) in routes {
        for failing in [false, true] {
            let mut fixture = Fixture::new(0);
            set_source(&mut fixture.state, failing);
            let response = fixture.get(path).await;
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "no-store",
                "{path}"
            );
            let status = response.status();
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            if failing {
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{path}");
                assert!(
                    body.is_empty(),
                    "{path}: errors must not expose private data"
                );
            } else {
                assert_eq!(status, StatusCode::OK, "{path}");
                let mut body: Value = serde_json::from_slice(&body).unwrap();
                if path == "/api/telemetry" {
                    let collected = body
                        .as_object_mut()
                        .unwrap()
                        .remove("collected_at")
                        .unwrap();
                    collected
                        .as_str()
                        .unwrap()
                        .parse::<chrono::DateTime<chrono::Utc>>()
                        .unwrap();
                }
                assert_eq!(
                    body,
                    json!({"api_version": API_VERSION, field: []}),
                    "{path}"
                );
            }
        }
    }
}
