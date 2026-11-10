use super::registry;
use crate::agents::AgentSpec;
use crate::config::spec::load_project_config_from;
use crate::session;
#[cfg(test)]
use crate::store::atomic_write;
use crate::telemetry::{BUCKET, Buckets};
use crate::worker::usage;
use anyhow::{Context, Result};
use chrono::{DateTime, DurationRound, TimeDelta, Utc};
use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct Session {
    project: String,
    id: String,
    entry: bool,
    family: String,
    model: Option<String>,
    created_at: DateTime<Utc>,
    closed_at: Option<DateTime<Utc>>,
    buckets: Buckets,
}

const HISTORY: TimeDelta = TimeDelta::days(30);

pub(crate) fn collect() -> Result<Vec<Session>> {
    let since = Utc::now()
        .checked_sub_signed(HISTORY)
        .context("telemetry history overflow")?
        .duration_trunc(BUCKET)?;
    let mut sessions = Vec::new();
    for entry in registry::entries()? {
        sessions.extend(project(&entry, since)?);
    }
    Ok(sessions)
}

// race accepted: close/supersede can move a session between these reads. Like the local panel,
// one snapshot can miss or double-count it; refreshing replaces that cached snapshot.
fn project(entry: &registry::Entry, since: DateTime<Utc>) -> Result<Vec<Session>> {
    let models = load_project_config_from(&entry.path)?.models;
    let make_session = |id,
                        entry_session,
                        agent: &str,
                        created_at,
                        (closed_at, buckets): (Option<DateTime<Utc>>, Buckets)|
     -> Result<Session> {
        let agent = AgentSpec::parse(agent, &models)?;
        Ok(Session {
            project: entry.name.as_str().to_owned(),
            id,
            entry: entry_session,
            family: agent.family().to_owned(),
            model: agent.model().map(str::to_owned),
            created_at,
            closed_at,
            buckets: buckets.since(since),
        })
    };
    let open = usage::collect(&entry.path, session::latest_lead(&entry.path)?)?
        .into_iter()
        .map(|open| {
            let buckets = match open.usage {
                Some(usage) => usage.buckets,
                None => Buckets::default(),
            };
            make_session(
                open.id,
                open.entry,
                &open.agent,
                open.created_at,
                (None, buckets),
            )
        });
    let closed = usage::closed_workers(&entry.path, since)?
        .into_iter()
        .chain(usage::superseded_leads(&entry.path, since)?)
        .map(|closed| {
            make_session(
                closed.id,
                closed.entry,
                &closed.agent,
                closed.created_at,
                (Some(closed.closed_at), closed.buckets),
            )
        });
    open.chain(closed).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::paths;
    use crate::telemetry::fixtures::{self, PI};
    use crate::test_support;
    use crate::test_support::telemetry::{archive, lead, pi_link, write};
    use serde_json::json;
    use std::fs;

    #[test]
    fn exports_latest_archived_and_superseded_usage_without_transcript_text() {
        let root = test_support::temp_test_path("telemetry-export");
        let entry = registry::Entry {
            name: registry::ProjectName::parse("demo").unwrap(),
            path: root.clone(),
        };
        fs::create_dir_all(&root).unwrap();
        let state = paths::project_dir(&root).unwrap();
        let transcript = root.join("transcript");
        let link = pi_link(&transcript);
        write(
            &transcript.join("session.jsonl"),
            &format!(
                "{}{}\n",
                PI,
                json!({"type":"message","timestamp":"2026-10-01T17:00:00Z","message":{"role":"user","content":"private prompt"}})
            ),
        );

        for started in ["2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z"] {
            lead(&root, started, link.clone());
        }
        archive(
            &root,
            "impl",
            "2026-10-01T00:00:00Z",
            "2026-10-03T00:00:00Z",
            Some(link.clone()),
        );
        archive(
            &root,
            "old",
            "2026-09-01T00:00:00Z",
            "2026-09-29T00:00:00Z",
            Some(link),
        );
        let since = "2026-09-30T00:00:00Z".parse().unwrap();
        let sessions = project(&entry, since).unwrap();
        assert_eq!(sessions.len(), 3);
        assert_eq!(
            sessions
                .iter()
                .filter(|session| session.closed_at.is_some())
                .count(),
            2
        );
        for session in &sessions {
            assert_eq!(session.project, "demo");
            assert_eq!(session.family, "pi");
            assert_eq!(session.model.as_deref(), Some("tencent/hy3"));
            assert_eq!(
                session
                    .buckets
                    .iter()
                    .map(|(_, tokens)| tokens)
                    .sum::<u64>(),
                fixtures::pi().total_tokens().unwrap()
            );
        }
        let clipped = project(&entry, "2026-10-01T12:00:00Z".parse().unwrap()).unwrap();
        assert!(clipped.iter().all(|session| {
            session
                .buckets
                .iter()
                .map(|(_, tokens)| tokens)
                .sum::<u64>()
                == 140
        }));
        let recent = project(&entry, "2026-10-02T12:00:00Z".parse().unwrap()).unwrap();
        assert_eq!(recent.len(), 2);
        assert!(
            recent
                .iter()
                .all(|session| session.buckets.iter().next().is_none())
        );
        let wire = serde_json::to_value(&sessions).unwrap();
        let keys = wire
            .as_array()
            .unwrap()
            .first()
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            [
                "buckets",
                "closed_at",
                "created_at",
                "entry",
                "family",
                "id",
                "model",
                "project"
            ]
        );
        assert!(!wire.to_string().contains("private prompt"));
        assert!(!wire.to_string().contains(root.as_str()));
        atomic_write(
            state
                .join("sessions")
                .join(&sessions.first().unwrap().id)
                .join("session.json"),
            "bad",
        )
        .unwrap();
        assert!(project(&entry, since).is_err());
        test_support::remove_workspace(&root).unwrap();
    }
}
