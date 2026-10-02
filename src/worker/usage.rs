use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::{session, telemetry, util::current_dir_utf8};

use super::{list::print_json, meta::report_path, snapshot::worker_snapshot};

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
        let Some(meta) = worker.meta else {
            let error = worker.read_error.context("missing worker metadata error")?;
            bail!(
                "worker {} metadata is unreadable; remove its directory to recover: {error}",
                worker.id
            );
        };
        let usage = match &meta.session_link {
            Some(link) => telemetry::read(
                link,
                &meta.project,
                &report_path(&worker.worker_dir),
                meta.created_at,
            )?,
            None => None,
        };
        sessions.push(SessionUsage {
            id: worker.id,
            role: meta.role.as_str(),
            agent: meta.agent,
            usage,
        });
    }
    if let Some(meta) = session::live_lead(&workspace)? {
        let usage = match &meta.session_link {
            Some(link) => telemetry::read(
                link,
                &meta.workspace,
                meta.brief.parent().context("lead brief has no parent")?,
                meta.created_at,
            )?,
            None => None,
        };
        sessions.push(SessionUsage {
            id: meta.id,
            role: "lead",
            agent: meta.agent,
            usage,
        });
    }
    print_json(&UsageOutput { sessions })
}
