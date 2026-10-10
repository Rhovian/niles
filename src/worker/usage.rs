use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::{
    session, store,
    telemetry::{self, Buckets, SessionLink},
    util::{current_dir_utf8, parse_timestamp_id},
};

use super::{
    WorkerSnapshot,
    list::print_json,
    meta::{WorkerMeta, meta_path, report_path},
    resolve::window_state,
    snapshot::worker_snapshot,
};

#[derive(Serialize)]
pub(crate) struct SessionUsage {
    pub(crate) id: String,
    pub(crate) role: &'static str,
    pub(crate) agent: String,
    pub(crate) usage: Option<telemetry::Usage>,
    /// Always `false` for the lead.
    #[serde(skip)]
    pub(crate) window_gone: bool,
    #[serde(skip)]
    pub(crate) created_at: DateTime<Utc>,
}

/// A worker that was closed, or a lead a later session superseded. Neither changes again, so
/// their buckets are cached.
pub(crate) struct ClosedSession {
    pub(crate) id: String,
    pub(crate) role: &'static str,
    pub(crate) agent: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) closed_at: DateTime<Utc>,
    pub(crate) buckets: Buckets,
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
            created_at: meta.created_at,
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
            created_at: meta.created_at,
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

/// Archived workers closed at or after `since` that recorded a `session_link`. Older archives
/// have no transcript to read, and also predate `role`, so they are skipped unparsed.
pub(crate) fn closed_workers(
    workspace: &Utf8Path,
    since: DateTime<Utc>,
) -> Result<Vec<ClosedSession>> {
    let mut closed = Vec::new();
    for (closed_at, dir) in store::worker_archives(workspace)? {
        if closed_at < since {
            continue;
        }
        let path = meta_path(&dir);
        let Some(body) = store::read_optional_json::<serde_json::Value>(&path)? else {
            continue;
        };
        let context = || format!("failed to parse {path}");
        let link: SessionLink = match body.get("session_link") {
            None | Some(serde_json::Value::Null) => continue,
            Some(link) => serde_json::from_value(link.clone()).with_context(context)?,
        };
        let meta: WorkerMeta = serde_json::from_value(body).with_context(context)?;
        // Codex finds the transcript by the report path its brief named: the live one.
        let buckets = telemetry::closed_buckets(&dir, || {
            let worker_dir = meta.brief.parent().context("worker brief has no parent")?;
            telemetry::read(
                &link,
                meta.agent_dir(),
                &report_path(worker_dir),
                meta.created_at,
            )
        })?;
        closed.push(ClosedSession {
            role: meta.role.as_str(),
            id: meta.id,
            agent: meta.agent,
            created_at: meta.created_at,
            closed_at,
            buckets,
        });
    }
    Ok(closed)
}

/// Leads whose session a later one superseded, at or after `since`. Each ended when the next
/// session began. The latest session is never one: its lead may not have started yet.
pub(crate) fn superseded_leads(
    workspace: &Utf8Path,
    since: DateTime<Utc>,
) -> Result<Vec<ClosedSession>> {
    let Some(latest) = session::latest_lead(workspace)? else {
        return Ok(Vec::new());
    };
    let latest = parse_timestamp_id(&latest.id).context("lead session id is not a timestamp")?;
    let dirs = session::session_dirs(workspace)?;
    let mut closed = Vec::new();
    for ((started, dir), (closed_at, _)) in dirs.iter().zip(dirs.iter().skip(1)) {
        if *started >= latest {
            break;
        }
        if *closed_at < since {
            continue;
        }
        let Some(meta) =
            store::read_optional_json::<session::SessionMeta>(&dir.join("session.json"))?
        else {
            continue;
        };
        let buckets = match &meta.session_link {
            Some(_) => telemetry::closed_buckets(dir, || lead_usage(&meta))?,
            None => Buckets::default(),
        };
        closed.push(ClosedSession {
            id: meta.id,
            role: "lead",
            agent: meta.agent,
            created_at: meta.created_at,
            closed_at: *closed_at,
            buckets,
        });
    }
    Ok(closed)
}
