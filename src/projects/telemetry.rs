use anyhow::Result;
use chrono::{DateTime, DurationRound, TimeDelta, Utc};
use serde::Serialize;

use super::registry;
use crate::{
    agents::{AgentSpec, ModelRoster},
    config::spec::load_project_config_from,
    session,
    telemetry::{BUCKET, Buckets},
    worker::usage,
};

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
    let since = (Utc::now() - HISTORY).duration_trunc(BUCKET)?;
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
    let mut sessions = Vec::new();
    for open in usage::collect(&entry.path, session::latest_lead(&entry.path)?)? {
        sessions.push(Session::from_open(entry, &models, open, since)?);
    }
    for closed in usage::closed_workers(&entry.path, since)?
        .into_iter()
        .chain(usage::superseded_leads(&entry.path, since)?)
    {
        sessions.push(Session::from_closed(entry, &models, closed, since)?);
    }
    Ok(sessions)
}

impl Session {
    fn from_open(
        project: &registry::Entry,
        models: &ModelRoster,
        open: usage::SessionUsage,
        since: DateTime<Utc>,
    ) -> Result<Self> {
        let agent = AgentSpec::parse(&open.agent, models)?;
        Ok(Self {
            project: project.name.as_str().to_owned(),
            id: open.id,
            entry: open.entry,
            family: agent.family().to_owned(),
            model: agent.model().map(str::to_owned),
            created_at: open.created_at,
            closed_at: None,
            buckets: match open.usage {
                Some(usage) => usage.buckets.since(since),
                None => Buckets::default(),
            },
        })
    }

    fn from_closed(
        project: &registry::Entry,
        models: &ModelRoster,
        closed: usage::ClosedSession,
        since: DateTime<Utc>,
    ) -> Result<Self> {
        let agent = AgentSpec::parse(&closed.agent, models)?;
        Ok(Self {
            project: project.name.as_str().to_owned(),
            id: closed.id,
            entry: closed.entry,
            family: agent.family().to_owned(),
            model: agent.model().map(str::to_owned),
            created_at: closed.created_at,
            closed_at: Some(closed.closed_at),
            buckets: closed.buckets.since(since),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn exports_latest_archived_and_superseded_usage_without_transcript_text() {
        let root = crate::test_support::temp_test_path("telemetry-export");
        let entry = registry::Entry {
            name: registry::ProjectName::parse("demo").unwrap(),
            path: root.clone(),
        };
        fs::create_dir_all(&root).unwrap();
        let state = crate::store::paths::project_dir(&root).unwrap();
        let transcript = root.join("transcript");
        fs::create_dir_all(&transcript).unwrap();
        fs::write(
            transcript.join("session.jsonl"),
            format!(
                "{}{}\n",
                crate::telemetry::fixtures::PI,
                json!({"type":"message","timestamp":"2026-10-01T17:00:00Z","message":{"role":"user","content":"private prompt"}})
            ),
        )
        .unwrap();
        let link = json!({"family":"pi", "session_dir":transcript});
        for day in [1, 2] {
            let at: DateTime<Utc> = format!("2026-10-0{day}T00:00:00Z").parse().unwrap();
            let id = crate::util::timestamp_id(&at);
            let dir = state.join("sessions").join(&id);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("session.json"), json!({
                "id":id, "entry_skill":"solo", "agent":"pi:tencent/hy3:high",
                "created_at":at, "workspace":root, "brief":dir.join("lead.md"), "session_link":link
            }).to_string()).unwrap();
        }
        let closed_at: DateTime<Utc> = "2026-10-03T00:00:00Z".parse().unwrap();
        let dir = state.join(format!(
            "worker/archive/impl-{}",
            crate::util::timestamp_id(&closed_at)
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("meta.json"),
            json!({
                "id":"impl", "skill":"worker", "agent":"pi:tencent/hy3:high",
                "created_at":"2026-10-01T00:00:00Z", "project":root, "window":"demo:impl",
                "launch":dir.join("launch.sh"), "brief":dir.join("brief.md"), "session_link":link
            })
            .to_string(),
        )
        .unwrap();
        let old_closed: DateTime<Utc> = "2026-09-29T00:00:00Z".parse().unwrap();
        let old_dir = state.join(format!(
            "worker/archive/old-{}",
            crate::util::timestamp_id(&old_closed)
        ));
        fs::create_dir_all(&old_dir).unwrap();
        let mut old_meta: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
        old_meta["id"] = json!("old");
        old_meta["created_at"] = json!("2026-09-01T00:00:00Z");
        fs::write(old_dir.join("meta.json"), old_meta.to_string()).unwrap();
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
                crate::telemetry::fixtures::pi().total_tokens()
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
        let keys = wire[0]
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
        fs::write(
            state
                .join("sessions")
                .join(&sessions[0].id)
                .join("session.json"),
            "bad",
        )
        .unwrap();
        assert!(project(&entry, since).is_err());
        crate::test_support::remove_workspace(&root).unwrap();
    }
}
