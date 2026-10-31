use super::{ApiError, AppState, AuthorizedUser, POLL, Session, Source, blocking};
use anyhow::Result;
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use isahc::{
    HttpClient,
    config::{Configurable, RedirectPolicy},
};
use std::{collections::HashSet, sync::Arc, time::Duration};
use web_push::{IsahcWebPushClient, WebPushClient, WebPushError};
mod store;
pub(crate) use store::Store;
use store::Subscription;
#[cfg(test)]
mod tests;
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn init(subject: &str) -> Result<()> {
    Store::create(crate::util::home_dir()?.join(".niles/remote"), subject)?;
    eprintln!("Web Push initialized; restart niles remote to enable notifications");
    Ok(())
}
pub(super) async fn vapid(State(state): State<AppState>) -> Result<Response, ApiError> {
    let Some(store) = state.push else {
        return Ok(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    Ok(Json(serde_json::json!({"public_key": store.public_key()?})).into_response())
}
pub(super) async fn subscribe(
    State(state): State<AppState>,
    Extension(AuthorizedUser(user)): Extension<AuthorizedUser>,
    Json(registration): Json<store::Registration>,
) -> Result<StatusCode, ApiError> {
    let Some(store) = state.push else {
        return Ok(StatusCode::SERVICE_UNAVAILABLE);
    };
    blocking(move || store.save(user, registration.subscription, registration.host)).await?;
    Ok(StatusCode::OK)
}
fn newly_waiting(previous: &mut HashSet<String>, sessions: Vec<Session>) -> Vec<Session> {
    let waiting: Vec<_> = sessions
        .into_iter()
        .filter(|session| session.waiting)
        .collect();
    let changed = waiting
        .iter()
        .filter(|session| !previous.contains(&session.id))
        .cloned()
        .collect();
    *previous = waiting.into_iter().map(|session| session.id).collect();
    changed
}
pub(super) async fn watch(source: Source, store: Arc<Store>) -> Result<()> {
    let client = IsahcWebPushClient::from(
        HttpClient::builder()
            .timeout(SEND_TIMEOUT)
            .redirect_policy(RedirectPolicy::None)
            .build()?,
    );
    let mut previous = HashSet::new();
    let mut timer = tokio::time::interval(POLL);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        timer.tick().await;
        let source = source.clone();
        let sessions =
            blocking(move || Ok(source()?.iter().map(Session::from_record).collect())).await?;
        for session in newly_waiting(&mut previous, sessions) {
            let list_store = store.clone();
            for device in blocking(move || list_store.subscriptions()).await? {
                let message = store.message(&device.subscription, &device.payload(&session)?)?;
                finish_delivery(&store, &device.subscription, client.send(message).await)?;
            }
        }
    }
}
fn finish_delivery(
    store: &Store,
    subscription: &Subscription,
    result: Result<(), WebPushError>,
) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(WebPushError::EndpointNotFound(_) | WebPushError::EndpointNotValid(_)) => {
            store.prune(subscription)
        }
        Err(error) => Err(error.into()),
    }
}
