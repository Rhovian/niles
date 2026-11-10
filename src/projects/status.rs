use super::{panels, registry, rows, windows};
use crate::cli::StatusLine;
#[cfg(feature = "remote")]
use crate::session::{self, SessionMeta};
#[cfg(feature = "remote")]
use crate::telemetry::SessionState;
use crate::theme::{self, StyleKey, StyleRender, Theme};
#[cfg(feature = "remote")]
use crate::worker;
use crate::worker::usage::SessionUsage;
use crate::{telemetry, tmux};
use anyhow::Result;
use camino::Utf8PathBuf;

pub(crate) struct Segment {
    pub label: String,
    pub state: Option<theme::State>,
    pub tokens: Option<u64>,
    pub age: Option<String>,
}

impl Segment {
    pub(crate) fn activity(&self) -> theme::State {
        match self.state {
            Some(theme::State::Running) => theme::State::Running,
            Some(theme::State::Starting) => theme::State::Starting,
            Some(theme::State::Idle | theme::State::Waiting) | None => theme::State::Idle,
        }
    }
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
            home(entries.len(), &sessions, &theme)
        }
        StatusLine::Projects => project_line(&theme),
        StatusLine::Sessions { session_name } => session_line(&session_name, &theme),
    }
}

#[cfg(feature = "remote")]
#[derive(Clone)]
pub(crate) struct ForegroundSession {
    pub project: String,
    pub meta: SessionMeta,
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
        let Some(meta) = session::latest_lead(&row.entry.path)? else {
            continue;
        };
        let Some(windows) = tmux::windows(&row.entry.name.session()?)? else {
            continue;
        };
        let labels = worker::window_labels(&row.entry.path)?;
        let worker_count = labels
            .iter()
            .filter(|(_, target)| {
                windows
                    .iter()
                    .any(|window| *target == format!("{}:{}", row.entry.name.as_str(), window.name))
            })
            .count();
        let lead = worker::lead_session(&meta, chrono::Utc::now())?;
        sessions.push(ForegroundSession {
            project: row.entry.name.as_str().to_owned(),
            meta,
            waiting: lead.state.displayed() == SessionState::Waiting,
            worker_count,
        });
    }
    Ok(sessions)
}

/// The home session's header: projects, agents, lost agents, and their total tokens.
fn home(projects: usize, sessions: &[SessionUsage], theme: &Theme) -> Result<String> {
    let lost = sessions
        .iter()
        .filter(|session| session.window_gone)
        .count();
    let live = sessions
        .iter()
        .filter(|session| !session.window_gone)
        .count();
    let tokens = sessions
        .iter()
        .filter_map(|session| session.usage.as_ref())
        .try_fold(0, |total, usage| {
            telemetry::sum([total, usage.total_tokens()?])
        })?;
    let count =
        |key, count: String, label| format!("{}{count}#[default] {label}", theme.style(key).tmux());
    let mut parts = vec![
        format!("{}NILES#[default]", theme.style(StyleKey::Accent).tmux()),
        count(StyleKey::Heading, projects.to_string(), "projects"),
        count(StyleKey::Running, live.to_string(), "agents"),
    ];
    if lost > 0 {
        parts.push(count(StyleKey::Lost, lost.to_string(), "lost"));
    }
    parts.push(count(StyleKey::Heading, rows::abbreviate(tokens), "tok"));
    Ok(parts.join("  "))
}

pub(crate) fn project(name: &str) -> Result<Option<Utf8PathBuf>> {
    Ok(registry::find(name)?.map(|entry| entry.path))
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
    use crate::telemetry::SessionState;
    use crate::telemetry::Usage;

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
            skill: "worker".parse().unwrap(),
            entry: false,
            agent: "codex".into(),
            state: SessionState::Unknown,
            usage: tokens.map(|input_tokens| Usage {
                input_tokens,
                ..Usage::default()
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
            home(4, &sessions, &theme).unwrap(),
            format!(
                "{accent}NILES#[default]  {heading}4#[default] projects  {running}2#[default] agents  {lost}1#[default] lost  {heading}3.4M#[default] tok"
            )
        );
        assert_eq!(
            home(4, &sessions[..2], &theme).unwrap(),
            format!(
                "{accent}NILES#[default]  {heading}4#[default] projects  {running}2#[default] agents  {heading}3.0M#[default] tok"
            )
        );
    }
}
