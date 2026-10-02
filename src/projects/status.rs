use anyhow::Result;
use chrono::{DateTime, Utc};

use super::{
    registry,
    rows::{self, Row, State},
};
use crate::{
    cli::StatusLine,
    session,
    telemetry::{SessionState, Usage},
    tmux, worker,
};
use camino::Utf8PathBuf;

pub(crate) struct Segment {
    pub label: String,
    pub model: Option<String>,
    pub glyph: Option<&'static str>,
    pub tokens: Option<u64>,
    pub age: Option<String>,
    pub highlighted: bool,
}

pub(crate) fn run(line: StatusLine, session_name: &str) -> Result<String> {
    let now = Utc::now();
    let segments = match line {
        StatusLine::Projects => from_rows(&rows::collect(registry::entries()?)?, session_name, now),
        StatusLine::Sessions => session_segments(session_name, now)?,
    };
    Ok(render(&segments))
}

fn project(name: &str) -> Result<Option<Utf8PathBuf>> {
    Ok(registry::entries()?
        .into_iter()
        .find(|entry| entry.name.as_str() == name)
        .map(|entry| entry.path))
}

fn from_rows(rows: &[Row], session_name: &str, now: DateTime<Utc>) -> Vec<Segment> {
    rows.iter()
        .filter_map(|row| {
            let (glyph, age) = match row.state {
                State::Running => ("●", None),
                State::Waiting(since) => ("⚠", since.map(|since| age(now, since))),
                State::Missing | State::NotRunning => return None,
            };
            Some(Segment {
                highlighted: row.entry.name.as_str() == session_name,
                label: row.entry.name.as_str().to_owned(),
                model: None,
                glyph: Some(glyph),
                tokens: None,
                age,
            })
        })
        .collect()
}

fn session_segments(session_name: &str, now: DateTime<Utc>) -> Result<Vec<Segment>> {
    let session_name = tmux::SessionName::new(session_name)?;
    let windows = tmux::windows(&session_name)?;
    let project = project(session_name.as_str())?;
    let (lead, workers) = match &project {
        Some(path) => (session::latest_lead(path)?, worker::status_workers(path)?),
        None => (None, Vec::new()),
    };
    windows
        .into_iter()
        .map(|window| {
            let (label, model, glyph, tokens, age) = if project.is_some() && window.name == "niles"
            {
                let model = lead
                    .as_ref()
                    .map(|lead| lead.model.clone().unwrap_or_else(|| lead.agent.clone()));
                let usage = lead.as_ref().map(worker::lead_usage).transpose()?.flatten();
                let (glyph, tokens) = usage_fields(usage.as_ref(), true);
                ("lead".to_owned(), model, glyph, tokens, None)
            } else if let Some(worker) = workers
                .iter()
                .find(|worker| worker.window == format!("{}:{}", session_name, window.name))
            {
                let (glyph, tokens) = usage_fields(worker.usage.as_ref(), false);
                (
                    worker.id.clone(),
                    Some(worker.model.clone()),
                    glyph,
                    tokens,
                    Some(age(now, worker.created_at)),
                )
            } else {
                (window.name, None, None, None, None)
            };
            Ok(Segment {
                label: format!("{}:{label}", window.index),
                model,
                glyph,
                tokens,
                age,
                highlighted: window.active,
            })
        })
        .collect()
}

fn usage_fields(usage: Option<&Usage>, lead: bool) -> (Option<&'static str>, Option<u64>) {
    match usage {
        Some(usage) => (
            usage.state.map(|state| match state {
                SessionState::Working => "●",
                SessionState::Waiting if lead => "⚠",
                SessionState::Waiting => "○",
            }),
            Some(usage.total_tokens()),
        ),
        None => (None, None),
    }
}

fn age(now: DateTime<Utc>, since: DateTime<Utc>) -> String {
    let minutes = (now - since).num_minutes().max(0);
    if minutes >= 60 {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}m")
    }
}

pub(crate) fn render(segments: &[Segment]) -> String {
    segments
        .iter()
        .map(|segment| {
            let tokens = segment.tokens.map(rows::abbreviate);
            let text = [
                Some(segment.label.as_str()),
                segment.model.as_deref(),
                segment.glyph,
                tokens.as_deref(),
                segment.age.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
            .replace('#', "##");
            if segment.highlighted {
                format!("#[reverse]{text}#[noreverse]")
            } else {
                text
            }
        })
        .collect::<Vec<_>>()
        .join(" │ ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_segments_render_lead_worker_plain_and_active() {
        let segment =
            |label: &str, model: Option<&str>, glyph, tokens, age: Option<&str>, highlighted| {
                Segment {
                    label: label.into(),
                    model: model.map(str::to_owned),
                    glyph,
                    tokens,
                    age: age.map(str::to_owned),
                    highlighted,
                }
            };
        assert_eq!(
            render(&[
                segment("0:lead", Some("opus"), Some("⚠"), Some(12_000), None, false),
                segment(
                    "1:w#parse",
                    Some("gpt-6"),
                    Some("●"),
                    Some(840_000),
                    Some("6m"),
                    true
                ),
                segment("2:shell", None, None, None, None, false),
            ]),
            "0:lead opus ⚠ 12k │ #[reverse]1:w##parse gpt-6 ● 840k 6m#[noreverse] │ 2:shell"
        );
    }
    #[test]
    fn projects_render_live_states_and_escape_names() {
        let now = Utc::now();
        let row = |name, state| Row {
            entry: registry::Entry {
                name: registry::ProjectName::parse(name).unwrap(),
                path: "/tmp".into(),
            },
            state,
            lead_tokens: None,
            workers: 0,
        };
        let rows = [
            row("api", State::Running),
            row(
                "wait",
                State::Waiting(Some(now - chrono::Duration::minutes(12))),
            ),
            row("unknown", State::Waiting(None)),
            row("gone", State::NotRunning),
        ];
        assert_eq!(
            render(&from_rows(&rows, "api", now)),
            "#[reverse]api ●#[noreverse] │ wait ⚠ 12m │ unknown ⚠"
        );
        let escaped = Segment {
            label: "a#b".into(),
            model: None,
            glyph: Some("●"),
            tokens: None,
            age: None,
            highlighted: false,
        };
        assert_eq!(render(&[escaped]), "a##b ●");
    }
}
