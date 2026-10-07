use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use serde::Serialize;

use crate::{session, telemetry, util::current_dir_utf8};

use super::{
    WorkerSnapshot, list::print_json, meta::report_path, resolve::window_state,
    snapshot::worker_snapshot,
};

#[derive(Serialize)]
pub(crate) struct SessionUsage {
    pub(crate) id: String,
    pub(crate) role: &'static str,
    pub(crate) agent: String,
    pub(crate) usage: Option<telemetry::Usage>,
    /// Always `false` for the lead, which is only collected while it runs.
    #[serde(skip)]
    pub(crate) window_gone: bool,
}

#[derive(Serialize)]
struct UsageOutput {
    sessions: Vec<SessionUsage>,
}

pub fn usage() -> Result<()> {
    let workspace = current_dir_utf8()?;
    print_json(&UsageOutput {
        sessions: collect(&workspace, session::live_lead(&workspace)?)?,
    })
}

pub(crate) fn collect(
    workspace: &Utf8Path,
    lead: Option<session::SessionMeta>,
) -> Result<Vec<SessionUsage>> {
    let mut sessions = Vec::new();
    for worker in worker_snapshot(workspace)? {
        let Some(meta) = worker.meta.as_ref() else {
            let error = worker.read_error.context("missing worker metadata error")?;
            bail!(
                "worker {} metadata is unreadable; remove its directory to recover: {error}",
                worker.id
            );
        };
        let usage = worker_usage(&worker)?;
        sessions.push(SessionUsage {
            id: worker.id,
            role: meta.role.as_str(),
            agent: meta.agent.clone(),
            usage,
            window_gone: window_state(meta).is_gone(),
        });
    }
    if let Some(meta) = lead {
        let usage = lead_usage(&meta)?;
        sessions.push(SessionUsage {
            id: meta.id,
            role: "lead",
            agent: meta.agent,
            usage,
            window_gone: false,
        });
    }
    Ok(sessions)
}

pub(crate) fn worker_usage(worker: &WorkerSnapshot) -> Result<Option<telemetry::Usage>> {
    let Some(meta) = worker.meta.as_ref() else {
        return Ok(None);
    };
    let Some(link) = meta.session_link.as_ref() else {
        return Ok(None);
    };
    telemetry::read(
        link,
        meta.agent_dir(),
        &report_path(&worker.worker_dir),
        meta.created_at,
    )
}

pub(crate) fn lead_usage(meta: &session::SessionMeta) -> Result<Option<telemetry::Usage>> {
    match &meta.session_link {
        Some(link) => telemetry::read(
            link,
            &meta.workspace,
            meta.brief.parent().context("lead brief has no parent")?,
            meta.created_at,
        ),
        None => Ok(None),
    }
}
