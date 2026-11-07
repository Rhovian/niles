use super::*;

#[tokio::test]
async fn telemetry_returns_a_versioned_snapshot_and_reports_collection_errors() {
    let mut fixture = Fixture::new(0);
    fixture.state.telemetry = Arc::new(|| Ok(Vec::new()));
    let response = fixture
        .app()
        .oneshot(request("/api/telemetry"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["api_version"], API_VERSION);
    assert_eq!(body["sessions"], json!([]));
    assert!(
        body["collected_at"]
            .as_str()
            .unwrap()
            .parse::<chrono::DateTime<chrono::Utc>>()
            .is_ok()
    );
    fixture.state.telemetry = Arc::new(|| anyhow::bail!("unreadable telemetry"));
    let response = fixture
        .app()
        .oneshot(request("/api/telemetry"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
}
