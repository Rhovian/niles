use anyhow::{Result, ensure};
use axum::{Json, http::StatusCode};
use serde::Deserialize;

use super::{ApiError, blocking, chat::Lookup};
use crate::tmux;

pub(super) const MAX_BODY_BYTES: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Message {
    text: MessageText,
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

pub(super) async fn send(
    lookup: Lookup,
    Json(message): Json<Message>,
) -> Result<StatusCode, ApiError> {
    Ok(blocking(move || {
        let Some(session) = lookup()? else {
            return Ok(StatusCode::NOT_FOUND);
        };
        let Some(pane) = session.target else {
            return Ok(StatusCode::SERVICE_UNAVAILABLE);
        };
        tmux::send_line(&pane, &message.text.0)?;
        Ok(StatusCode::OK)
    })
    .await?)
}
