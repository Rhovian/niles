use anyhow::Result;
use chrono::{DateTime, Utc};

use super::{
    registry,
    rows::{self, Row, State},
    windows::{self, age},
};
use crate::{
    cli::StatusLine,
    theme::{self, StyleKey, StyleRender, Theme},
    tmux,
};
use camino::Utf8PathBuf;

pub(crate) struct Segment {
    pub label: String,
    pub model: Option<String>,
    pub state: Option<theme::State>,
    pub tokens: Option<u64>,
    pub age: Option<String>,
    pub highlighted: bool,
}

pub(crate) fn run(line: StatusLine) -> Result<String> {
    let theme = Theme::load()?;
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
    Ok(render(&segments, &theme))
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
            let (state, age) = match row.state {
                State::Running => (theme::State::Running, None),
                State::Waiting(since) => {
                    (theme::State::Waiting, since.map(|since| age(now, since)))
                }
                State::Missing | State::NotRunning => return None,
            };
            Some(Segment {
                highlighted: row.entry.name.as_str() == session_name,
                label: row.entry.name.as_str().to_owned(),
                model: None,
                state: Some(state),
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

pub(crate) fn render(segments: &[Segment], theme: &Theme) -> String {
    segments
        .iter()
        .map(|segment| {
            let base = if segment.highlighted {
                theme.style(StyleKey::Pill).tmux()
            } else {
                String::new()
            };
            let glyph = segment.state.map(|state| {
                let (glyph, style) = theme.state(state);
                format!(
                    "{}{}#[default]{base}",
                    style.tmux(),
                    glyph.replace('#', "##")
                )
            });
            let tokens = segment.tokens.map(rows::abbreviate);
            let text = [
                Some(segment.label.replace('#', "##")),
                segment.model.as_ref().map(|s| s.replace('#', "##")),
                glyph,
                tokens,
                segment.age.clone(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
            if segment.highlighted {
                format!("{base}{text}#[default]")
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
        let theme = Theme::parse(None).unwrap();
        let pill = theme.style(StyleKey::Pill).tmux();
        let running = theme.style(StyleKey::Running).tmux();
        let waiting = theme.style(StyleKey::Waiting).tmux();
        let segment =
            |label: &str, model: Option<&str>, state, tokens, age: Option<&str>, highlighted| {
                Segment {
                    label: label.into(),
                    model: model.map(str::to_owned),
                    state,
                    tokens,
                    age: age.map(str::to_owned),
                    highlighted,
                }
            };
        assert_eq!(
            render(
                &[
                    segment(
                        "0:lead",
                        Some("opus"),
                        Some(theme::State::Waiting),
                        Some(12_000),
                        None,
                        false
                    ),
                    segment(
                        "1:w#parse",
                        Some("gpt-6"),
                        Some(theme::State::Running),
                        Some(840_000),
                        Some("6m"),
                        true
                    ),
                    segment("2:shell", None, None, None, None, false),
                ],
                &theme
            ),
            format!(
                "0:lead opus {waiting}⚠#[default] 12k │ {pill}1:w##parse gpt-6 {running}●#[default]{pill} 840k 6m#[default] │ 2:shell"
            )
        );
    }
    #[test]
    fn projects_render_live_states_and_escape_names() {
        let theme = Theme::parse(None).unwrap();
        let pill = theme.style(StyleKey::Pill).tmux();
        let running = theme.style(StyleKey::Running).tmux();
        let waiting = theme.style(StyleKey::Waiting).tmux();
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
            render(&from_rows(&rows, "api", now), &theme),
            format!(
                "{pill}api {running}●#[default]{pill}#[default] │ wait {waiting}⚠#[default] 12m │ unknown {waiting}⚠#[default]"
            )
        );
        let escaped = Segment {
            label: "a#b".into(),
            model: None,
            state: Some(theme::State::Running),
            tokens: None,
            age: None,
            highlighted: false,
        };
        assert_eq!(
            render(&[escaped], &theme),
            format!("a##b {running}●#[default]")
        );
    }
}
