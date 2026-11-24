use super::{ApiError, message};
use crate::config::user::FileMcp;
use anyhow::{Context, Result, ensure};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
};
use serde::Serialize;
use std::net::Ipv4Addr;
use std::sync::Arc;
use tokio::net::TcpListener;

mod oidc;
mod tools;

use oidc::{Verifier, https_url};
use tools::LeadTools;

const DEFAULT_PORT: u16 = 8766;
const ENDPOINT: &str = "/mcp";
const METADATA: &str = "/.well-known/oauth-protected-resource/mcp";

struct Auth {
    verifier: Verifier,
    challenge: HeaderValue,
    invalid_token: HeaderValue,
    metadata: Metadata,
    authority: String,
}

#[derive(Clone, Serialize)]
struct Metadata {
    resource: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorization_servers: Option<[String; 1]>,
    bearer_methods_supported: [&'static str; 1],
}

impl Auth {
    fn new(verifier: Verifier, config: FileMcp) -> Result<Self> {
        let resource = https_url(&config.resource)?;
        ensure!(
            resource.path() == ENDPOINT
                && resource.query().is_none()
                && resource.as_str() == config.resource,
            "remote.mcp.resource must be a canonical HTTPS URL with path {ENDPOINT}"
        );
        ensure!(
            config
                .interactive_issuer
                .as_ref()
                .is_none_or(|issuer| config.issuers.contains_key(issuer)),
            "remote.mcp.interactive_issuer must name a trusted issuer"
        );
        let metadata_url = resource.join(METADATA)?;
        let challenge = format!("Bearer resource_metadata=\"{metadata_url}\"");
        Ok(Self {
            verifier,
            challenge: challenge.parse()?,
            invalid_token: format!("{challenge}, error=\"invalid_token\"").parse()?,
            metadata: Metadata {
                resource: config.resource,
                authorization_servers: config.interactive_issuer.map(|issuer| [issuer]),
                bearer_methods_supported: ["header"],
            },
            authority: resource.authority().to_owned(),
        })
    }
}

pub(super) async fn run(config: FileMcp) -> Result<()> {
    let port = match config.port {
        Some(port) => port,
        None => DEFAULT_PORT,
    };
    let verifier = Verifier::load(&config).await?;
    let app = router(Auth::new(verifier, config)?, LeadTools::new());
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .context("bind remote MCP listener")?;
    eprintln!("remote MCP listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await.context("serve remote MCP")
}

fn router(auth: Auth, tools: LeadTools) -> Router {
    let auth = Arc::new(auth);
    let transport = StreamableHttpServerConfig::default()
        .with_allowed_hosts([auth.authority.clone()])
        .enforce_origin_validation()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_max_request_body_bytes(message::MAX_BODY_BYTES);
    let service = StreamableHttpService::new(
        move || Ok(tools.clone()),
        Arc::new(NeverSessionManager::default()),
        transport,
    );
    let metadata = auth.metadata.clone();
    Router::new()
        .route_service(ENDPOINT, service)
        .route_layer(middleware::from_fn_with_state(auth, authenticate))
        .route(METADATA, get(move || async { Json(metadata) }))
}

async fn authenticate(State(auth): State<Arc<Auth>>, mut request: Request, next: Next) -> Response {
    let grants = match bearer(request.headers()) {
        Some(token) => match auth.verifier.verify(token).await {
            Ok(Some(grants)) if !grants.is_empty() => grants,
            Ok(Some(_)) => return StatusCode::FORBIDDEN.into_response(),
            Ok(None) => return challenge(auth.invalid_token.clone()),
            Err(error) => return ApiError(error).into_response(),
        },
        None if request.headers().contains_key(header::AUTHORIZATION) => {
            return challenge(auth.invalid_token.clone());
        }
        None => return challenge(auth.challenge.clone()),
    };
    request.headers_mut().remove(header::AUTHORIZATION);
    request.extensions_mut().insert(Arc::new(grants));
    next.run(request).await
}

fn challenge(value: HeaderValue) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, value)],
    )
        .into_response()
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let Ok(value) = values.next()?.to_str() else {
        return None;
    };
    let (scheme, token) = value.split_once(' ')?;
    (values.next().is_none() && scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty())
        .then_some(token.trim_start_matches(' '))
}
