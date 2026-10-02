use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::{session, telemetry, util::current_dir_utf8};

use super::{WorkerSnapshot, list::print_json, meta::report_path, snapshot::worker_snapshot};

#[derive(Serialize)]
struct SessionUsage {
    id: String,
    role: &'static str,
    agent: String,
    usage: Option<telemetry::Usage>,
}

#[derive(Serialize)]
struct UsageOutput {
    sessions: Vec<SessionUsage>,
}

pub fn usage() -> Result<()> {
    let workspace = current_dir_utf8()?;
    let mut sessions = Vec::new();
    for worker in worker_snapshot(&workspace)? {
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
        });
    }
    if let Some(meta) = session::live_lead(&workspace)? {
        let usage = lead_usage(&meta)?;
        sessions.push(SessionUsage {
            id: meta.id,
            role: "lead",
            agent: meta.agent,
            usage,
        });
    }
    print_json(&UsageOutput { sessions })
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
