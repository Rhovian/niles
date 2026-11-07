use super::{
    ApiError, AppState, AuthorizedUser, POLL, Session, Source, blocking,
    chat::Chat,
    feed::{Follower, mobile_text},
};
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
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use web_push::{IsahcWebPushClient, WebPushClient, WebPushError};
mod store;
pub(crate) use store::Store;
pub(super) use store::{DeviceId, Subscription};
#[cfg(test)]
mod tests;
use crate::{projects::status::ForegroundSession, telemetry::transcript::TextRole};

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
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Reason {
    Waiting,
    OperatorMessage,
}
impl Reason {
    fn text(self) -> &'static str {
        match self {
            Self::Waiting => "Agent waiting for your input",
            Self::OperatorMessage => "Operator sent a message",
        }
    }
}
struct Observed {
    follower: Result<Follower>,
    waiting: bool,
    pending: Vec<Pending>,
}
// Interactive Claude 2.1.312, Codex 0.165.0 and Pi 1.4.0 persist different
// surrounding whitespace; all normalize pasted CRLF to LF. Keep interior spacing.
fn echoed_text(text: &str) -> String {
    text.trim().replace("\r\n", "\n")
}
struct Pending {
    id: uuid::Uuid,
    text: String,
    sender: Sender,
}
pub(super) struct Notification {
    session: Session,
    reason: Reason,
    excluded: Option<Sender>,
}
impl Notification {
    fn includes(&self, device: &store::Device) -> bool {
        !self
            .excluded
            .as_ref()
            .is_some_and(|sender| device.is_sender(sender))
    }
}
#[derive(Clone)]
pub(super) struct Sender {
    pub user: String,
    pub device: DeviceId,
}
#[derive(Default)]
pub(super) struct Observer(Mutex<Tracker>);
impl Observer {
    fn lock(&self) -> Result<MutexGuard<'_, Tracker>> {
        self.0
            .lock()
            .map_err(|_| anyhow::anyhow!("push observer poisoned"))
    }
    pub(super) fn prepare(&self, chat: Chat, text: &str, sender: Sender) -> Result<uuid::Uuid> {
        let mut tracker = self.lock()?;
        let id = uuid::Uuid::new_v4();
        let session = chat
            .status
            .clone()
            .ok_or_else(|| anyhow::anyhow!("push receipt requires a lead session"))?;
        let observed = tracker.observe(&session, Ok(chat));
        if observed.follower.is_ok() {
            observed.pending.push(Pending {
                id,
                text: echoed_text(text),
                sender,
            });
        }
        Ok(id)
    }
    pub(super) fn cancel(&self, id: uuid::Uuid) -> Result<()> {
        for observed in self.lock()?.sessions.values_mut() {
            observed.pending.retain(|pending| pending.id != id);
        }
        Ok(())
    }
}
#[derive(Default)]
struct Tracker {
    sessions: HashMap<String, Observed>,
    notifications: Vec<Notification>,
}
impl Tracker {
    fn observe(&mut self, session: &Session, chat: Result<Chat>) -> &mut Observed {
        let observed = self
            .sessions
            .entry(session.id.clone())
            .or_insert_with(|| Observed {
                follower: Ok(Follower::default()),
                waiting: false,
                pending: Vec::new(),
            });
        if let Ok(follower) = &mut observed.follower {
            match chat.and_then(|chat| follower.read_new(&chat.transcript)) {
                Ok(turns) => {
                    for text in turns
                        .into_iter()
                        .filter(|turn| turn.role == TextRole::User)
                        .filter_map(mobile_text)
                    {
                        let text = echoed_text(&text);
                        let index = observed
                            .pending
                            .iter()
                            .position(|pending| pending.text == text);
                        self.notifications.push(Notification {
                            session: session.clone(),
                            reason: Reason::OperatorMessage,
                            excluded: index.map(|index| observed.pending.remove(index).sender),
                        });
                    }
                }
                Err(error) => {
                    // Invalid transcript state stops this session's observation, as SSE does.
                    eprintln!("push session {} stopped: {error:#}", session.id);
                    observed.follower = Err(error);
                    observed.pending.clear();
                }
            }
        }
        if session.waiting && !observed.waiting {
            self.notifications.push(Notification {
                session: session.clone(),
                reason: Reason::Waiting,
                excluded: None,
            });
        }
        observed.waiting = session.waiting;
        observed
    }
    fn changes(&mut self, records: Vec<ForegroundSession>) -> Vec<Notification> {
        let mut current = HashSet::new();
        for record in records {
            let session = Session::from_record(&record);
            self.observe(&session, Chat::lead(&record));
            current.insert(session.id);
        }
        self.sessions.retain(|id, _| current.contains(id));
        std::mem::take(&mut self.notifications)
    }
}
pub(super) async fn watch(
    source: Source,
    store: Arc<Store>,
    observer: Arc<Observer>,
) -> Result<()> {
    let client = IsahcWebPushClient::from(
        HttpClient::builder()
            .timeout(SEND_TIMEOUT)
            .redirect_policy(RedirectPolicy::None)
            .build()?,
    );
    let mut timer = tokio::time::interval(POLL);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        timer.tick().await;
        let source = source.clone();
        let observer = observer.clone();
        let notifications = blocking(move || Ok(observer.lock()?.changes(source()?))).await?;
        for notification in notifications {
            let list_store = store.clone();
            for device in blocking(move || list_store.subscriptions()).await? {
                if !notification.includes(&device) {
                    continue;
                }
                let session = &notification.session;
                let message = store.message(
                    &device.subscription,
                    &device.payload(session, notification.reason)?,
                    device.topic(session)?,
                )?;
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
