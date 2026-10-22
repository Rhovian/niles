use super::{Session, Source, resolve, session_id};
use crate::{projects::status, telemetry::transcript::TranscriptSource, tmux::TmuxTarget, worker};
use anyhow::{Context, Result};
use std::sync::Arc;

pub(super) struct Chat {
    pub id: String,
    pub transcript: TranscriptSource,
    pub target: Option<TmuxTarget>,
    pub status: Option<Session>,
}
pub(super) type Lookup = Arc<dyn Fn() -> Result<Option<Chat>> + Send + Sync>;

pub(super) fn lead(source: Source, id: String) -> Lookup {
    Arc::new(move || resolve(&source, &id)?.as_ref().map(Chat::lead).transpose())
}
pub(super) fn worker(project: String, id: String) -> Lookup {
    Arc::new(move || {
        let Some(path) = status::project(&project)? else {
            return Ok(None);
        };
        Ok(worker::chat_target(&path, &id)?.map(|target| Chat {
            id: format!("{project}/{id}"),
            transcript: target.transcript,
            target: target.window.as_ref().map(TmuxTarget::window),
            status: None,
        }))
    })
}
impl Chat {
    fn lead(record: &status::ForegroundSession) -> Result<Self> {
        Ok(Self {
            id: session_id(record),
            transcript: TranscriptSource {
                link: record.meta.session_link.clone(),
                workspace: record.meta.workspace.clone(),
                needle: record
                    .meta
                    .brief
                    .parent()
                    .context("session brief has no parent")?
                    .to_owned(),
                created_at: record.meta.created_at,
            },
            target: record
                .meta
                .lead_pane
                .as_ref()
                .map(|pane| TmuxTarget::pane(pane))
                .transpose()?,
            status: Some(Session::from_record(record)),
        })
    }
}
