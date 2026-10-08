use anyhow::Result;
use chrono::{DateTime, Utc};

use super::{
    panels, registry,
    rows::{self, Row, State},
    windows,
};
use crate::{
    cli::StatusLine,
    telemetry::Usage,
    theme::{self, StyleKey, StyleRender, Theme},
    tmux,
    worker::usage::SessionUsage,
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
        StatusLine::Home => {
            let entries = registry::entries()?;
            let mut sessions = Vec::new();
            for entry in &entries {
                sessions.extend(panels::open_sessions(entry)?);
            }
            return Ok(home(entries.len(), &sessions, &theme));
        }
        StatusLine::Projects { session_name } => {
            from_rows(&rows::collect(registry::entries()?)?, &session_name)
        }
        StatusLine::Sessions {
            session_name,
            window_index,
        } => session_segments(&session_name, window_index, now)?,
    };
    Ok(render(&segments, &theme))
}

/// The home session's header: projects, agents, lost agents, and their total tokens.
fn home(projects: usize, sessions: &[SessionUsage], theme: &Theme) -> String {
    let lost = sessions
        .iter()
        .filter(|session| session.window_gone)
        .count();
    let tokens = sessions
        .iter()
        .filter_map(|session| session.usage.as_ref())
        .map(Usage::total_tokens)
        .sum();
    let count =
        |key, count: String, label| format!("{}{count}#[default] {label}", theme.style(key).tmux());
    let mut parts = vec![
        format!("{}NILES#[default]", theme.style(StyleKey::Accent).tmux()),
        count(StyleKey::Heading, projects.to_string(), "projects"),
        count(
            StyleKey::Running,
            (sessions.len() - lost).to_string(),
            "agents",
        ),
    ];
    if lost > 0 {
        parts.push(count(StyleKey::Lost, lost.to_string(), "lost"));
    }
    parts.push(count(StyleKey::Heading, rows::abbreviate(tokens), "tok"));
    parts.join("  ")
}

fn project(name: &str) -> Result<Option<Utf8PathBuf>> {
    Ok(registry::entries()?
        .into_iter()
        .find(|entry| entry.name.as_str() == name)
        .map(|entry| entry.path))
}

fn from_rows(rows: &[Row], session_name: &str) -> Vec<Segment> {
    rows.iter()
        .filter_map(|row| {
            if matches!(row.state, State::Missing | State::NotRunning) {
                return None;
            }
            Some(Segment {
                highlighted: row.entry.name.as_str() == session_name,
                label: row.entry.name.as_str().to_owned(),
                model: None,
                state: None,
                tokens: None,
                age: None,
            })
        })
        .collect()
}

fn session_segments(name: &str, active_index: u32, now: DateTime<Utc>) -> Result<Vec<Segment>> {
    let session_name = tmux::SessionName::new(name)?;
    let project = project(session_name.as_str())?;
    Ok(
        windows::session_agents(&session_name, project.as_deref(), now)?
            .into_iter()
            .flat_map(|agents| agents.windows)
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
    fn home_counts_live_and_lost_and_abbreviates_tokens() {
        let theme = Theme::parse(None).unwrap();
        let [accent, heading, running, lost] = [
            StyleKey::Accent,
            StyleKey::Heading,
            StyleKey::Running,
            StyleKey::Lost,
        ]
        .map(|key| theme.style(key).tmux());
        let session = |tokens: Option<u64>, window_gone| SessionUsage {
            id: "w".into(),
            role: "worker",
            agent: "codex".into(),
            usage: tokens.map(|input_tokens| Usage {
                input_tokens,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: None,
                reasoning_tokens: None,
                last_turn_at: None,
                state: None,
                estimated_cost_usd: None,
            }),
            window_gone,
        };
        let sessions = [
            session(Some(3_000_000), false),
            session(None, false),
            session(Some(400_000), true),
        ];
        assert_eq!(
            home(4, &sessions, &theme),
            format!(
                "{accent}NILES#[default]  {heading}4#[default] projects  {running}2#[default] agents  {lost}1#[default] lost  {heading}3.4M#[default] tok"
            )
        );
        assert_eq!(
            home(4, &sessions[..2], &theme),
            format!(
                "{accent}NILES#[default]  {heading}4#[default] projects  {running}2#[default] agents  {heading}3.0M#[default] tok"
            )
        );
    }

    #[test]
    fn projects_render_live_names_and_escape_names() {
        let theme = Theme::parse(None).unwrap();
        let pill = theme.style(StyleKey::Pill).tmux();
        let running = theme.style(StyleKey::Running).tmux();
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
            render(&from_rows(&rows, "api"), &theme),
            format!("{pill}api#[default] │ wait │ unknown")
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
