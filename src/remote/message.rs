use anyhow::{Result, ensure};
use axum::{Json, http::StatusCode};
use serde::Deserialize;

use super::{ApiError, blocking, chat::Lookup, push};
use crate::tmux;
use std::sync::Arc;

pub(super) const MAX_BODY_BYTES: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Message {
    text: MessageText,
    sender_device: Option<push::DeviceId>,
}

#[derive(Deserialize)]
#[serde(try_from = "String")]
struct MessageText(String);

impl TryFrom<String> for MessageText {
    type Error = anyhow::Error;

    fn try_from(text: String) -> Result<Self> {
        ensure!(!text.trim().is_empty(), "message must not be blank");
        ensure!(
            !text.contains(['\x1b', '\0']),
            "message cannot contain ESC or NUL: bracketed paste and process arguments cannot carry them"
        );
        Ok(Self(text))
    }
}

pub(super) struct Sender {
    pub user: String,
    pub observer: Arc<push::Observer>,
}

pub(super) async fn send(
    lookup: Lookup,
    Json(message): Json<Message>,
    sender: Option<Sender>,
) -> Result<StatusCode, ApiError> {
    Ok(blocking(move || {
        let Some(session) = lookup()? else {
            return Ok(StatusCode::NOT_FOUND);
        };
        deliver(session, message, sender, tmux::send_line)
    })
    .await?)
}

pub(super) fn deliver(
    session: super::chat::Chat,
    message: Message,
    sender: Option<Sender>,
    paste: impl FnOnce(&crate::tmux::TmuxTarget, &str) -> Result<()>,
) -> Result<StatusCode> {
    let origin = match (sender, message.sender_device) {
        (Some(sender), Some(device)) => Some((
            sender.observer,
            push::Sender {
                user: sender.user,
                device,
            },
        )),
        _ => None,
    };
    let Some(pane) = session.target.clone() else {
        return Ok(StatusCode::SERVICE_UNAVAILABLE);
    };
    let receipt = match origin {
        Some((observer, sender)) => Some((
            observer.prepare(session, &message.text.0, sender)?,
            observer,
        )),
        None => None,
    };
    // Publish the receipt before pasting, and release the observer lock during tmux I/O.
    let result = paste(&pane, &message.text.0);
    if result.is_err()
        && let Some((id, observer)) = receipt
    {
        observer.cancel(id)?;
    }
    result?;
    Ok(StatusCode::OK)
}
