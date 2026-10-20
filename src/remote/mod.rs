use std::{convert::Infallible, net::Ipv4Addr, sync::Arc, time::Duration};

use anyhow::{Result, ensure};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use serde::Serialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    config::user,
    projects::status::{self, ForegroundSession},
};

mod feed;
mod message;
pub(crate) mod push;
#[cfg(test)]
mod tests;

const API_VERSION: u32 = 1;
const SUBSCRIPTION_BODY_BYTES: usize = 4096;
const POLL: Duration = Duration::from_secs(1);
type Source = Arc<dyn Fn() -> Result<Vec<ForegroundSession>> + Send + Sync>;

#[derive(Clone)]
struct AppState {
    users: Arc<Vec<String>>,
    source: Source,
    push: Option<Arc<push::Store>>,
}

impl AppState {
    fn new(users: Vec<String>, source: Source) -> Result<Self> {
        ensure!(!users.is_empty(), "remote.allowed_users must not be empty");
        ensure!(
            users.iter().all(|user| !user.is_empty()
                && user.trim() == user
                && !user.chars().any(char::is_control)),
            "remote.allowed_users contains an invalid login"
        );
        Ok(Self {
            users: Arc::new(users),
            source,
            push: None,
        })
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
                family,
                entry_skill: record.meta.entry_skill.clone(),
            },
            waiting: record.waiting,
            worker_count: record.worker_count,
        }
    }
}

fn session_id(record: &ForegroundSession) -> String {
    format!("{}:{}", record.project, record.meta.id)
}

fn resolve(source: &Source, id: &str) -> Result<Option<ForegroundSession>> {
    Ok(source()?
        .into_iter()
        .find(|record| session_id(record) == id))
}

// The runtime is created only when this sync CLI entry point is invoked.
#[tokio::main]
pub(crate) async fn run(port: u16) -> Result<()> {
    let users =
        blocking(|| Ok(user::FileConfig::load(&user::path()?)?.remote.allowed_users)).await?;
    let mut state = AppState::new(users, Arc::new(status::live_sessions))?;
    state.push = blocking(push::Store::load_optional).await?.map(Arc::new);
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    eprintln!("remote listening on {}", listener.local_addr()?);
    let notifier = state.push.clone();
    let source = state.source.clone();
    let users = state.users.clone();
    let server = axum::serve(listener, router(state)).into_future();
    if let Some(store) = notifier {
        tokio::select! {
            result = server => result?,
            result = push::watch(source, store, users) => result?,
        }
    } else {
        server.await?;
    }
    Ok(())
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/push/vapid", get(push::vapid))
        .route(
            "/api/push/subscribe",
            post(push::subscribe).layer(DefaultBodyLimit::max(SUBSCRIPTION_BODY_BYTES)),
        )
        .route("/api/sessions", get(sessions))
        .route("/api/sessions/{id}/events", get(events))
        .route(
            "/api/sessions/{id}/message",
            post(message::send).layer(DefaultBodyLimit::max(message::MAX_BODY_BYTES)),
        )
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, authorize))
}

#[derive(Clone)]
struct AuthorizedUser(String);

async fn authorize(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let mut logins = request.headers().get_all("tailscale-user-login").iter();
    let login = logins
        .next()
        .and_then(|value| match value.to_str() {
            Ok(login) => Some(login),
            Err(_) => None,
        })
        .filter(|login| state.users.iter().any(|user| user == login))
        .filter(|_| logins.next().is_none())
        .map(str::to_owned);
    let mut response = if let Some(login) = login {
        request.extensions_mut().insert(AuthorizedUser(login));
        next.run(request).await
    } else {
        StatusCode::FORBIDDEN.into_response()
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(job).await?
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

async fn sessions(State(state): State<AppState>) -> Result<Response, ApiError> {
    let sessions = blocking(move || {
        Ok(state.source.as_ref()()?
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
) -> Result<Response, ApiError> {
    let source = state.source.clone();
    let initial =
        blocking(move || resolve(&source, &id)?.map(feed::Feed::open).transpose()).await?;
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
        if let Err(error) = pump(state.source, feed, sender).await {
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
    source: Source,
    mut feed: feed::Feed,
    sender: mpsc::Sender<Result<Event, Infallible>>,
) -> Result<()> {
    let mut interval = tokio::time::interval(POLL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
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
