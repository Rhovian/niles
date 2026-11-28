use super::{
    auth::{Auth, authenticate},
    message,
    oidc::{Verifier, https_url},
};
use crate::config::user::FileMcp;
use anyhow::{Context, Result, ensure};
use axum::middleware;
use axum::routing::get;
use axum::{Json, Router};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
};
use serde::Serialize;
use std::net::Ipv4Addr;
use std::sync::Arc;
use tokio::net::TcpListener;

mod tools;

pub(super) use tools::LeadTools;

const DEFAULT_PORT: u16 = 8766;
pub(super) const ENDPOINT: &str = "/mcp";
pub(super) const METADATA: &str = "/.well-known/oauth-protected-resource/mcp";

#[derive(Clone, Serialize)]
struct Metadata {
    resource: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorization_servers: Option<[String; 1]>,
    bearer_methods_supported: [&'static str; 1],
}

pub(super) async fn run(verifier: Arc<Verifier>, config: FileMcp) -> Result<()> {
    let port = match config.port {
        Some(port) => port,
        None => DEFAULT_PORT,
    };
    let app = router(verifier, config, LeadTools::new())?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .context("bind remote MCP listener")?;
    eprintln!("remote MCP listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await.context("serve remote MCP")
}

pub(super) fn router(verifier: Arc<Verifier>, config: FileMcp, tools: LeadTools) -> Result<Router> {
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
            .as_deref()
            .is_none_or(|issuer| verifier.trusts(issuer)),
        "remote.mcp.interactive_issuer must name a trusted issuer"
    );
    let auth = Auth::new(
        verifier,
        resource.clone(),
        Some(resource.join(METADATA)?.as_str()),
    )?;
    let transport = StreamableHttpServerConfig::default()
        .with_allowed_hosts([resource.authority().to_owned()])
        .enforce_origin_validation()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_max_request_body_bytes(message::MAX_BODY_BYTES);
    let service = StreamableHttpService::new(
        move || Ok(tools.clone()),
        Arc::new(NeverSessionManager::default()),
        transport,
    );
    let metadata = Metadata {
        resource: config.resource,
        authorization_servers: config.interactive_issuer.map(|issuer| [issuer]),
        bearer_methods_supported: ["header"],
    };
    Ok(Router::new()
        .route_service(ENDPOINT, service)
        .route_layer(middleware::from_fn_with_state(auth, authenticate))
        .route(METADATA, get(move || async { Json(metadata) })))
}
