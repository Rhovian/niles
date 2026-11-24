use super::*;

#[test]
fn discovery_requires_the_configured_issuer_and_https_keys() {
    for (issuer, jwks_uri, error) in [
        (
            WORKLOAD,
            "https://login.example.test/keys",
            "issuer mismatch",
        ),
        (
            INTERACTIVE,
            "http://login.example.test/keys",
            "expected an HTTPS URL",
        ),
    ] {
        let discovery = Discovery {
            issuer: issuer.into(),
            jwks_uri: jwks_uri.into(),
        };
        assert!(
            discovery
                .keys_url(INTERACTIVE)
                .unwrap_err()
                .to_string()
                .contains(error),
            "{issuer} {jwks_uri}"
        );
    }
}

#[tokio::test]
async fn refresh_retains_last_good_keys_and_survives_request_cancellation() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::time::timeout;

    async fn receive(listener: &TcpListener) -> BufReader<TcpStream> {
        timeout(FETCH_TIMEOUT, async {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            loop {
                assert_ne!(stream.read_line(&mut line).await.unwrap(), 0);
                if line == "\r\n" {
                    return stream;
                }
                line.clear();
            }
        })
        .await
        .unwrap()
    }

    async fn reply(mut stream: BufReader<TcpStream>, status: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let old = SigningKey::new();
    let mut issuer = old.issuer(Vec::new());
    issuer.jwks_uri = format!("http://{}/keys", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    issuer.client = HttpClient::builder()
        .timeout(FETCH_TIMEOUT)
        .proxy(None)
        .build()
        .unwrap();
    let issuer = Arc::new(issuer);
    let initial = issuer.keys().await.unwrap();
    assert_eq!(initial.len(), 1);
    {
        let mut cache = issuer.cache.lock().await;
        let stale = Instant::now().checked_sub(KEYS_TTL).unwrap();
        cache.loaded_at = stale;
        cache.attempted_at = stale;
    }
    let refresh = |issuer: &Arc<Issuer>| {
        let issuer = issuer.clone();
        tokio::spawn(async move { issuer.keys().await })
    };

    let x25519 = json!({"kty":"OKP", "crv":"X25519", "x":""});
    let unusable = json!({"keys": [x25519]}).to_string();
    for (status, document) in [
        ("503 Service Unavailable", ""),
        ("200 OK", unusable.as_str()),
    ] {
        let attempt = refresh(&issuer);
        reply(receive(&listener).await, status, document).await;
        assert!(Arc::ptr_eq(&initial, &attempt.await.unwrap().unwrap()));
        assert!(Arc::ptr_eq(&initial, &issuer.keys().await.unwrap()));
        issuer.cache.lock().await.attempted_at = Instant::now().checked_sub(RETRY_DELAY).unwrap();
    }

    let attempt = refresh(&issuer);
    let stream = receive(&listener).await;
    attempt.abort();
    assert!(attempt.await.unwrap_err().is_cancelled());
    let new = SigningKey::new();
    let document = json!({"keys": [
        {"kty":"EC", "crv":"secp256k1", "x":"", "y":""},
        new.key,
        x25519,
    ]});
    reply(stream, "200 OK", &document.to_string()).await;
    let keys = issuer.keys().await.unwrap();
    assert_eq!(
        serde_json::to_value(keys.as_ref()).unwrap(),
        json!([new.key])
    );

    issuer.cache.lock().await.loaded_at = Instant::now().checked_sub(MAX_KEY_AGE).unwrap();
    assert!(issuer.keys().await.is_err());
}
