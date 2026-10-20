use anyhow::Result;

use super::{panels, registry, rows, windows};
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
    pub state: Option<theme::State>,
    pub tokens: Option<u64>,
    pub age: Option<String>,
}

pub(crate) fn run(line: StatusLine) -> Result<String> {
    let theme = Theme::load()?;
    match line {
        StatusLine::Home => {
            let entries = registry::entries()?;
            let mut sessions = Vec::new();
            for entry in &entries {
                sessions.extend(panels::open_sessions(entry)?);
            }
            Ok(home(entries.len(), &sessions, &theme))
        }
        StatusLine::Projects { .. } => project_line(&theme),
        StatusLine::Sessions { session_name, .. } => session_line(&session_name, &theme),
    }
}

#[cfg(feature = "remote")]
#[derive(Clone)]
pub(crate) struct ForegroundSession {
    pub project: String,
    pub meta: crate::session::SessionMeta,
    pub waiting: bool,
    pub worker_count: usize,
}

#[cfg(feature = "remote")]
pub(crate) fn live_sessions() -> Result<Vec<ForegroundSession>> {
    let mut sessions = Vec::new();
    for row in rows::collect(registry::entries()?)? {
        if matches!(row.state, rows::State::Missing | rows::State::NotRunning) {
            continue;
        }
        let Some(meta) = crate::session::latest_lead(&row.entry.path)? else {
            continue;
        };
        let Some(windows) = tmux::windows(&row.entry.name.session()?)? else {
            continue;
        };
        let labels = crate::worker::window_labels(&row.entry.path)?;
        let worker_count = labels
            .iter()
            .filter(|(_, target)| {
                windows
                    .iter()
                    .any(|window| *target == format!("{}:{}", row.entry.name.as_str(), window.name))
            })
            .count();
        sessions.push(ForegroundSession {
            project: row.entry.name.as_str().to_owned(),
            meta,
            waiting: matches!(row.state, rows::State::Waiting(_)),
            worker_count,
        });
    }
    Ok(sessions)
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

fn project_line(theme: &Theme) -> Result<String> {
    let mut labels = Vec::new();
    for entry in registry::entries()? {
        if entry.path.is_dir() && entry.has_live_session()? {
            let name = entry.name.as_str();
            labels.push(selected_label(
                name,
                &format!("#{{==:#{{session_name}},{name}}}"),
                theme,
            ));
        }
    }
    Ok(labels.join(" │ "))
}

fn session_line(name: &str, theme: &Theme) -> Result<String> {
    let session = tmux::SessionName::new(name)?;
    let project = project(session.as_str())?;
    Ok(windows::labels(&session, project.as_deref())?
        .into_iter()
        .map(|(index, label)| {
            selected_label(
                &format!("{index}:{label}"),
                &format!("#{{==:#{{window_index}},{index}}}"),
                theme,
            )
        })
        .collect::<Vec<_>>()
        .join(" │ "))
}

// tmux 3.8 format.c:457 expands cached job output on every redraw; selection must
// remain a format, not a value captured by the asynchronous job.
// https://github.com/tmux/tmux/blob/3.8/format.c#L457
fn selected_label(label: &str, condition: &str, theme: &Theme) -> String {
    let pill = theme.style(StyleKey::Pill).tmux().replace(',', "#,");
    let text = label.replace('#', "##");
    format!("#{{?{condition},{pill},}}{text}#[default]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_is_a_format_and_labels_are_literal() {
        let theme = Theme::parse(None).unwrap();
        let pill = theme.style(StyleKey::Pill).tmux().replace(',', "#,");
        assert_eq!(
            selected_label("1:w#{pane_id},#[fg=red]", "#{==:#{window_index},1}", &theme),
            format!(
                "#{{?#{{==:#{{window_index}},1}},{pill},}}1:w##{{pane_id}},##[fg=red]#[default]"
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
            state: crate::telemetry::SessionState::Unknown,
            usage: tokens.map(|input_tokens| Usage {
                input_tokens,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: None,
                reasoning_tokens: None,
                last_turn_at: None,
                estimated_cost_usd: None,
                buckets: Default::default(),
                prompt_tokens: None,
                context_window: None,
            }),
            window_gone,
            created_at: chrono::DateTime::UNIX_EPOCH,
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
}
