use anyhow::{Result, ensure};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use super::{ApiError, AppState, blocking, resolve};
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
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(message): Json<Message>,
) -> Result<StatusCode, ApiError> {
    Ok(blocking(move || {
        let Some(session) = resolve(&state.source, &id)? else {
            return Ok(StatusCode::NOT_FOUND);
        };
        let Some(pane) = session.meta.lead_pane else {
            return Ok(StatusCode::SERVICE_UNAVAILABLE);
        };
        tmux::send_line(&tmux::TmuxTarget::pane(&pane)?, &message.text.0)?;
        Ok(StatusCode::OK)
    })
    .await?)
}
