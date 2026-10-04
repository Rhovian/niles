use anyhow::Result;
use chrono::{DateTime, Utc};

use super::{
    registry,
    rows::{self, Row, State},
    windows::{self, age},
};
use crate::{cli::StatusLine, tmux};
use camino::Utf8PathBuf;

pub(crate) struct Segment {
    pub label: String,
    pub model: Option<String>,
    pub glyph: Option<&'static str>,
    pub tokens: Option<u64>,
    pub age: Option<String>,
    pub highlighted: bool,
}

pub(crate) fn run(line: StatusLine) -> Result<String> {
    let now = Utc::now();
    let segments = match line {
        StatusLine::Projects { session_name } => {
            from_rows(&rows::collect(registry::entries()?)?, &session_name, now)
        }
        StatusLine::Sessions {
            session_name,
            window_index,
        } => session_segments(&session_name, window_index, now)?,
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

fn session_segments(name: &str, active_index: u32, now: DateTime<Utc>) -> Result<Vec<Segment>> {
    let session_name = tmux::SessionName::new(name)?;
    let project = project(session_name.as_str())?;
    Ok(
        windows::session_agents(&session_name, project.as_deref(), now)?
            .windows
            .into_iter()
            .map(|window| Segment {
                label: format!("{}:{}", window.index, window.segment.label),
                highlighted: window.index == active_index,
                ..window.segment
            })
            .collect(),
    )
}

impl Segment {
    pub(super) fn text(&self) -> String {
        let tokens = self.tokens.map(rows::abbreviate);
        [
            Some(self.label.as_str()),
            self.model.as_deref(),
            self.glyph,
            tokens.as_deref(),
            self.age.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
    }
}

pub(crate) fn render(segments: &[Segment]) -> String {
    segments
        .iter()
        .map(|segment| {
            let text = segment.text().replace('#', "##");
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
