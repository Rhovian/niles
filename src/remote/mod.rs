use crate::config::user::{
    self,
    Access::{Operate, Read},
    FileWeb,
};
use crate::projects::navigation::{self, Project};
use crate::projects::status::ForegroundSession;
use crate::projects::telemetry;
use anyhow::{Context, Result, bail, ensure};
use auth::{Auth, authenticate};
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::sse::{Event, KeepAlive};
use axum::response::{IntoResponse, Response, Sse};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use oidc::{Grants, Verifier, https_url};
use serde::Serialize;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task;
use tokio::time::{self, MissedTickBehavior};
use tokio_stream::wrappers::ReceiverStream;

mod auth;
mod chat;
mod control;
mod feed;
mod mcp;
mod message;
mod oidc;
pub(crate) mod push;
#[cfg(test)]
mod tests;

const API_VERSION: u32 = 1;
const SUBSCRIPTION_BODY_BYTES: usize = 4096;
const POLL: Duration = Duration::from_secs(1);
type Source = Arc<dyn Fn() -> Result<Vec<ForegroundSession>> + Send + Sync>;
type Collection<T> = Arc<dyn Fn(&dyn Fn(&str) -> bool) -> Result<Vec<T>> + Send + Sync>;

#[derive(Clone)]
struct AppState {
    source: Collection<ForegroundSession>,
    projects: Collection<Project>,
    push: Option<Arc<push::Store>>,
    push_observer: Arc<push::Observer>,
    telemetry: Collection<telemetry::Session>,
}

impl AppState {
    fn new(source: Collection<ForegroundSession>) -> Self {
        Self {
            source,
            projects: Arc::new(|allows| navigation::collect_for(allows)),
            push: None,
            push_observer: Arc::default(),
            telemetry: Arc::new(|allows| telemetry::collect_for(allows)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct Session {
    id: String,
    project: String,
    agent: Agent,
    waiting: bool,
    worker_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct Agent {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    family: String,
    entry_skill: String,
}

impl Session {
    fn from_record(record: &ForegroundSession) -> Self {
        let family = match &record.meta.agent_family {
            Some(family) => family.clone(),
            None => match record.meta.agent.split_once(':') {
                Some((family, _)) => family.to_owned(),
                None => record.meta.agent.clone(),
            },
        };
        Self {
            id: session_id(record),
            project: record.project.clone(),
            agent: Agent {
                model: record.meta.model.clone(),
                family,
                entry_skill: record.meta.entry_skill.to_string(),
            },
            waiting: record.waiting,
            worker_count: record.worker_count,
        }
    }
}

fn session_id(record: &ForegroundSession) -> String {
    format!("{}:{}", record.project, record.meta.id)
}

pub(crate) async fn run(port: u16) -> Result<()> {
    let config = blocking(|| user::FileConfig::load(&user::path()?))
        .await?
        .remote;
    let verifier = Arc::new(Verifier::load(config.issuers, config.introspection).await?);
    match (config.web, config.mcp) {
        (Some(web), Some(mcp)) => tokio::select! {
            result = run_web(port, web, verifier.clone()) => result,
            result = mcp::run(verifier, mcp) => result,
        },
        (Some(web), None) => run_web(port, web, verifier).await,
        (None, Some(mcp)) => mcp::run(verifier, mcp).await,
        (None, None) => bail!("configure remote.web or remote.mcp"),
    }
}

async fn run_web(port: u16, web: FileWeb, verifier: Arc<Verifier>) -> Result<()> {
    let resource = https_url(&web.resource)?;
    ensure!(
        resource.path().ends_with("/api")
            && resource.query().is_none()
            && resource.as_str() == web.resource,
        "remote.web.resource must be a canonical HTTPS URL ending in /api"
    );
    ensure!(
        !web.bind.is_unspecified(),
        "remote.web.bind must name a specific IPv4 address"
    );
    let auth = Auth::new(verifier.clone(), resource, None)?;
    let mut state = AppState::new(Arc::new(|allows| navigation::sessions_for(allows)));
    state.push = blocking(push::Store::load_optional).await?.map(Arc::new);
    let listener = TcpListener::bind((web.bind, port))
        .await
        .context("bind remote listener")?;
    eprintln!("remote listening on {}", listener.local_addr()?);
    let notifier = state.push.clone();
    let source: Source = Arc::new(navigation::push_sessions);
    let observer = state.push_observer.clone();
    let server = axum::serve(listener, router(state, auth)).into_future();
    if let Some(store) = notifier {
        tokio::select! {
            result = server => result.context("serve remote API")?,
            result = push::watch(source, store, observer, verifier) => result.context("watch Web Push notifications")?,
        }
    } else {
        server.await?;
    }
    Ok(())
}

fn router(state: AppState, auth: Auth) -> Router {
    Router::new()
        .route("/api/push/vapid", get(push::vapid))
        .route(
            "/api/push/subscribe",
            post(push::subscribe).layer(DefaultBodyLimit::max(SUBSCRIPTION_BODY_BYTES)),
        )
        .route("/api/projects", get(projects))
        .route("/api/telemetry", get(telemetry))
        .route(
            "/api/projects/{project}/session",
            put(control::start).delete(control::stop),
        )
        .route(
            "/api/projects/{project}/workers/{id}",
            delete(control::close),
        )
        .route(
            "/api/projects/{project}/workers/{id}/events",
            get(worker_events),
        )
        .route(
            "/api/projects/{project}/workers/{id}/message",
            post(worker_message).layer(DefaultBodyLimit::max(message::MAX_BODY_BYTES)),
        )
        .route("/api/sessions", get(sessions))
        .route("/api/sessions/{id}/events", get(events))
        .route(
            "/api/sessions/{id}/message",
            post(lead_message).layer(DefaultBodyLimit::max(message::MAX_BODY_BYTES)),
        )
        .with_state(state)
        .layer(middleware::from_fn_with_state(auth.clone(), authenticate))
        .route(
            "/api/auth/token",
            post(auth::exchange)
                .layer(DefaultBodyLimit::max(2048))
                .with_state(auth),
        )
}

async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    task::spawn_blocking(job).await?
}

struct ApiError(anyhow::Error);
impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self(error)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        eprintln!("remote request: {:#}", self.0);
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}

async fn projects(
    State(state): State<AppState>,
    Extension(grants): Extension<Arc<Grants>>,
) -> Result<Response, ApiError> {
    let projects =
        blocking(move || (state.projects)(&|project| grants.allows(project, Read))).await?;
    Ok(Json(serde_json::json!({"api_version": API_VERSION, "projects": projects})).into_response())
}

async fn telemetry(
    State(state): State<AppState>,
    Extension(grants): Extension<Arc<Grants>>,
) -> Result<Response, ApiError> {
    let sessions =
        blocking(move || (state.telemetry)(&|project| grants.allows(project, Read))).await?;
    Ok(Json(serde_json::json!({
        "api_version": API_VERSION, "collected_at": chrono::Utc::now(), "sessions": sessions,
    }))
    .into_response())
}

async fn sessions(
    State(state): State<AppState>,
    Extension(grants): Extension<Arc<Grants>>,
) -> Result<Response, ApiError> {
    let sessions = blocking(move || {
        Ok((state.source)(&|project| grants.allows(project, Read))?
            .iter()
            .map(Session::from_record)
            .collect::<Vec<_>>())
    })
    .await?;
    Ok(Json(serde_json::json!({"api_version": API_VERSION, "sessions": sessions})).into_response())
}

async fn events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(grants): Extension<Arc<Grants>>,
) -> Result<Response, ApiError> {
    let expires = grants.expires;
    stream(
        chat::lead(state.source, id, move |name| grants.allows(name, Read)),
        expires,
    )
    .await
}

async fn worker_events(
    Path((project, id)): Path<(String, String)>,
    Extension(grants): Extension<Arc<Grants>>,
) -> Result<Response, ApiError> {
    if !grants.allows(&project, Read) {
        return Ok(StatusCode::FORBIDDEN.into_response());
    }
    stream(chat::worker(project, id), grants.expires).await
}

async fn lead_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(grants): Extension<Arc<Grants>>,
    message: Json<message::Message>,
) -> Result<StatusCode, ApiError> {
    let sender = state.push.map(|_| message::Sender {
        principal: grants.principal.clone(),
        observer: state.push_observer,
    });
    message::send(
        chat::lead(state.source, id, move |name| grants.allows(name, Operate)),
        message,
        sender,
    )
    .await
}
async fn worker_message(
    Path((project, id)): Path<(String, String)>,
    Extension(grants): Extension<Arc<Grants>>,
    message: Json<message::Message>,
) -> Result<StatusCode, ApiError> {
    if !grants.allows(&project, Operate) {
        return Ok(StatusCode::FORBIDDEN);
    }
    message::send(chat::worker(project, id), message, None).await
}
async fn stream(lookup: chat::Lookup, expires: u64) -> Result<Response, ApiError> {
    let elapsed = SystemTime::UNIX_EPOCH
        .elapsed()
        .context("read stream expiry clock")?;
    let lifetime = Duration::from_secs(expires).saturating_sub(elapsed);
    if lifetime.is_zero() {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    }
    let deadline = time::Instant::now()
        .checked_add(lifetime)
        .context("stream expiry exceeds clock range")?;
    let initial_lookup = lookup.clone();
    let initial = blocking(move || initial_lookup()?.map(feed::Feed::open).transpose()).await?;
    let Some((feed, snapshot)) = initial else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let (sender, receiver) = mpsc::channel::<Result<Event, Infallible>>(8);
    sender
        .send(Ok(Event::default()
            .event("snapshot")
            .json_data(snapshot)
            .map_err(anyhow::Error::from)?))
        .await
        .map_err(anyhow::Error::from)?;
    tokio::spawn(async move {
        if let Ok(Err(error)) = time::timeout_at(deadline, pump(lookup, feed, sender)).await {
            eprintln!("remote stream: {error:#}");
        }
    });
    Ok(Sse::new(ReceiverStream::new(receiver))
        .keep_alive(KeepAlive::default())
        .into_response())
}

// axum frames SSE and tokio bounds delivery/cancels on receiver closure; neither polls stores.
// https://docs.rs/axum/0.8.9/axum/response/sse/index.html
// https://docs.rs/tokio/1.54.0/tokio/sync/mpsc/struct.Sender.html#method.closed
async fn pump(
    source: chat::Lookup,
    mut feed: feed::Feed,
    sender: mpsc::Sender<Result<Event, Infallible>>,
) -> Result<()> {
    let mut interval = time::interval(POLL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! { _ = sender.closed() => return Ok(()), _ = interval.tick() => {} }
        let source = source.clone();
        let result = blocking(move || {
            let events = feed.poll(&source)?;
            Ok((feed, events))
        })
        .await?;
        let (updated, events) = result;
        feed = updated;
        let Some(events) = events else {
            return Ok(());
        };
        for event in events {
            if sender.send(Ok(event)).await.is_err() {
                return Ok(());
            }
        }
    }
}
