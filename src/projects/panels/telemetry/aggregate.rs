use std::{cmp::Reverse, collections::BTreeMap, iter};

use clap::ValueEnum;

use super::{
    Window,
    collect::{EventKind, Project, Session},
};
use crate::worker::WorkerRole;

/// The newest events the panel lists.
const EVENTS: usize = 8;

pub(super) struct Dashboard<'a> {
    pub(super) window: Window,
    /// Sessions with tokens in the window, and every live one.
    pub(super) sessions: usize,
    pub(super) spawns: usize,
    pub(super) wakes: usize,
    pub(super) projects: Vec<(&'a str, u64)>,
    /// Most tokens first.
    pub(super) models: Vec<ModelRow<'a>>,
    pub(super) bins: Vec<u64>,
    /// Highest share of the context window first; those with no known window last.
    pub(super) context: Vec<ContextRow<'a>>,
    /// Newest first.
    pub(super) events: Vec<(&'a str, &'a super::collect::Event)>,
}

pub(super) struct ModelRow<'a> {
    pub(super) agent: &'a str,
    /// Counts by recorded role, leads first. A role with no session is absent.
    pub(super) roles: Vec<(&'static str, usize)>,
    pub(super) tokens: u64,
}

pub(super) struct ContextRow<'a> {
    pub(super) project: &'a str,
    pub(super) session: &'a Session,
    pub(super) prompt_tokens: u64,
    pub(super) percent: Option<u64>,
}

pub(super) fn dashboard(projects: &[Project], window: Window) -> Dashboard<'_> {
    let mut bins = vec![0; window.count.cast_unsigned() as usize];
    let mut models = BTreeMap::<&str, ModelRow<'_>>::new();
    let mut project_tokens = Vec::new();
    let mut context = Vec::new();
    let mut sessions = 0;
    for project in projects {
        let mut spent = 0;
        for session in &project.sessions {
            let mut tokens = 0;
            for (bucket, bucket_tokens) in session.buckets.iter() {
                if let Some(bin) = window.bin_of(bucket) {
                    bins[bin] += bucket_tokens;
                    tokens += bucket_tokens;
                }
            }
            spent += tokens;
            if let Some(live) = &session.live
                && let Some(prompt_tokens) = live.prompt_tokens
            {
                context.push(ContextRow {
                    project: &project.name,
                    session,
                    prompt_tokens,
                    percent: live
                        .context_window
                        .and_then(|window| (prompt_tokens * 100).checked_div(window)),
                });
            }
            if tokens == 0 && session.live.is_none() {
                continue;
            }
            sessions += 1;
            let row = models.entry(&session.agent).or_insert_with(|| ModelRow {
                agent: &session.agent,
                roles: Vec::new(),
                tokens: 0,
            });
            row.tokens += tokens;
            match row.roles.iter_mut().find(|(role, _)| *role == session.role) {
                Some((_, count)) => *count += 1,
                None => row.roles.push((session.role, 1)),
            }
        }
        project_tokens.push((project.name.as_str(), spent));
    }
    let mut models = models.into_values().collect::<Vec<_>>();
    for row in &mut models {
        row.roles.sort_by_key(|(role, _)| {
            iter::once("lead")
                .chain(
                    WorkerRole::value_variants()
                        .iter()
                        .map(|role| role.as_str()),
                )
                .position(|known| known == *role)
        });
    }
    models.sort_by_key(|row| Reverse(row.tokens));
    context.sort_by_key(|row| Reverse((row.percent, row.prompt_tokens)));
    let mut events = projects
        .iter()
        .flat_map(|project| {
            project
                .events
                .iter()
                .map(|event| (project.name.as_str(), event))
        })
        .filter(|(_, event)| window.contains(event.at))
        .collect::<Vec<_>>();
    let count = |predicate: fn(EventKind) -> bool| {
        events
            .iter()
            .filter(|(_, event)| predicate(event.kind))
            .count()
    };
    let spawns = count(|kind| kind == EventKind::Spawned);
    let wakes = count(|kind| matches!(kind, EventKind::Reported(_)));
    events.sort_by_key(|(_, event)| Reverse(event.at));
    events.truncate(EVENTS);
    Dashboard {
        window,
        sessions,
        spawns,
        wakes,
        projects: project_tokens,
        models,
        bins,
        context,
        events,
    }
}
