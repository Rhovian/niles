use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::BTreeSet;

use super::super::registry;
use crate::skills::SkillName;
use crate::{
    agents::{AgentSpec, ModelCatalogue},
    session,
    telemetry::Buckets,
    wake::WakeKind,
    watch,
    worker::usage::{self, ClosedSession},
};

pub(super) struct Project {
    pub(super) name: String,
    pub(super) sessions: Vec<Session>,
    pub(super) events: Vec<Event>,
    pub(super) catalogue_errors: BTreeSet<String>,
}

pub(super) struct Session {
    pub(super) id: String,
    pub(super) skill: SkillName,
    pub(super) entry: bool,
    pub(super) agent: String,
    pub(super) buckets: Buckets,
    /// Present for live workers and the running lead.
    pub(super) live: Option<Live>,
}

pub(super) struct Live {
    /// `None` for a session with no per-turn record.
    pub(super) prompt_tokens: Option<u64>,
    pub(super) context_window: Option<u64>,
}

pub(super) struct Event {
    pub(super) at: DateTime<Utc>,
    pub(super) id: String,
    pub(super) kind: EventKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EventKind {
    Spawned,
    Reported(WakeKind),
    Closed,
}

/// Everything the dashboard shows of one project: its live sessions, those closed at or after
/// `since`, and its events.
///
/// race accepted: a worker `close` archives between listing live workers and archives is missed
/// or counted twice for one render; the next one is correct.
pub(super) fn project(
    entry: &registry::Entry,
    lead_running: bool,
    since: DateTime<Utc>,
    models: &ModelCatalogue,
) -> Result<Project> {
    let mut sessions = Vec::new();
    let mut catalogue_errors = BTreeSet::new();
    let mut events = Vec::new();
    // The latest lead is read live whether or not it runs; only a running one has context.
    for open in usage::collect(&entry.path, session::latest_lead(&entry.path)?)? {
        let spec = AgentSpec::parse(&open.agent)?;
        let lead = open.entry;
        if !lead {
            events.push(event(open.created_at, &open.id, EventKind::Spawned));
        }
        let (buckets, prompt_tokens, transcript_window) = match open.usage {
            Some(usage) => (usage.buckets, usage.prompt_tokens, usage.context_window),
            None => (Buckets::default(), None, None),
        };
        let live = if lead && !lead_running {
            None
        } else {
            Some(Live {
                prompt_tokens,
                context_window: match transcript_window {
                    Some(window) => Some(window),
                    None => match catalogue_window(&spec, models) {
                        Ok(window) => window,
                        Err(error) => {
                            catalogue_errors.insert(error.to_string());
                            None
                        }
                    },
                },
            })
        };
        sessions.push(Session {
            id: open.id,
            skill: open.skill,
            entry: open.entry,
            agent: open.agent,
            buckets,
            live,
        });
    }
    for closed in usage::closed_workers(&entry.path, since)? {
        events.push(event(closed.created_at, &closed.id, EventKind::Spawned));
        events.push(event(closed.closed_at, &closed.id, EventKind::Closed));
        sessions.push(closed_session(closed));
    }
    sessions.extend(
        usage::superseded_leads(&entry.path, since)?
            .into_iter()
            .map(closed_session),
    );
    for (_, dir) in session::session_dirs(&entry.path)? {
        for (at, id, kind) in watch::delivered_reports(&dir)? {
            events.push(event(at, &id, EventKind::Reported(kind)));
        }
    }
    Ok(Project {
        name: entry.name.as_str().to_owned(),
        sessions,
        events,
        catalogue_errors,
    })
}

fn event(at: DateTime<Utc>, id: &str, kind: EventKind) -> Event {
    Event {
        at,
        id: id.to_owned(),
        kind,
    }
}

fn closed_session(closed: ClosedSession) -> Session {
    Session {
        id: closed.id,
        skill: closed.skill,
        entry: closed.entry,
        agent: closed.agent,
        buckets: closed.buckets,
        live: None,
    }
}

/// The CLI's window for the model `agent` was launched with; `None` when it names no model.
pub(super) fn catalogue_window(spec: &AgentSpec, models: &ModelCatalogue) -> Result<Option<u64>> {
    spec.model()
        .map(|model| models.context_window(spec.family(), model))
        .transpose()
        .map(Option::flatten)
}
