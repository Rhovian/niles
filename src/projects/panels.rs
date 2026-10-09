use std::fmt::Write;

use anyhow::Result;
use clap::ValueEnum;
use ratatui::style::Style;

use super::{registry, rows};
use crate::{
    agents::ModelRoster,
    config::spec::{PROJECT_CONFIG_FILES, load_project_config_from},
    session,
    telemetry::SessionState,
    theme::{State, StyleKey, StyleRender, Theme},
    tmux,
    worker::usage::{self, SessionUsage},
    workspace_manifest,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Panel {
    Config,
    Telemetry,
    Help,
}

impl Panel {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Telemetry => "telemetry",
            Self::Help => "help",
        }
    }
}

pub(crate) fn render(panel: Panel) -> Result<()> {
    let theme = Theme::load()?;
    let mut text = String::new();
    let entries = registry::entries()?;
    match panel {
        Panel::Help => text.push_str(&help_text(entries.is_empty(), &theme)),
        Panel::Config => text.push_str(&config_panel(&entries, &theme)?),
        Panel::Telemetry => {
            let projects = entries
                .into_iter()
                .map(|entry| {
                    let sessions = open_sessions(&entry)?;
                    Ok((entry.name.as_str().to_owned(), sessions))
                })
                .collect::<Result<Vec<_>>>()?;
            text.push_str(&telemetry(&projects, &theme)?);
        }
    }
    print!("{text}");
    Ok(())
}

/// The project's workers, and its lead while it runs.
pub(super) fn open_sessions(entry: &registry::Entry) -> Result<Vec<SessionUsage>> {
    let session = entry.name.session()?;
    let lead = if tmux::project_session(&session)?.as_deref() == Some(entry.path.as_str())
        && tmux::lead_running(&session)?
    {
        session::latest_lead(&entry.path)?
    } else {
        None
    };
    usage::collect(&entry.path, lead)
}

const FIRST_RUN: &str = "niles

Coordinates coding agents from different model families in tmux.

Get started
  r    register a project directory, in the explorer on the left
  ↵    start its lead and show it here

Key bindings for the home view: https://github.com/Rhovian/niles/blob/main/docs/setup.md
";

const KEYS: &str = "Choose a project on the left.

  ↵      open                 → ←    expand / collapse
  [ ]    previous / next project
  ; '    previous / next window
  r      register a project
  c      close a worker or project
  ?      this help            esc    back

  {running} running   {waiting} waiting

With the bindings in docs/setup.md, M-[ M-] M-; M-' do the same from any pane.
";

fn help_text(first_run: bool, theme: &Theme) -> String {
    if first_run {
        FIRST_RUN
            .split('\n')
            .map(|line| match line {
                "niles" | "Get started" => theme.style(StyleKey::Heading).paint(line),
                _ => line.to_owned(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        let (waiting, style) = theme.state(State::Waiting);
        KEYS.replace(
            "{running}",
            &theme.state(State::Running).1.paint(theme.spinner(0)),
        )
        .replace("{waiting}", &style.paint(waiting))
    }
}

const MANIFEST: &str = ".niles/manifest.yaml";
const ERROR_WIDTH: usize = 68;
const MODEL_WIDTH: usize = 40;

struct Cell {
    text: String,
    style: Style,
    right: bool,
}
impl Cell {
    fn left(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
            right: false,
        }
    }
    fn right(text: impl Into<String>, style: Style) -> Self {
        Self {
            right: true,
            ..Self::left(text, style)
        }
    }
}

fn table(rows: &[Vec<Cell>]) -> (Vec<String>, usize) {
    let mut widths = vec![0; rows.iter().map(Vec::len).fold(0, usize::max)];
    for row in rows {
        for (cell, width) in row.iter().zip(&mut widths) {
            *width = (*width).max(cell.text.chars().count());
        }
    }
    let lines = rows
        .iter()
        .map(|row| {
            row.iter()
                .zip(&widths)
                .map(|(cell, width)| {
                    let text = if cell.right {
                        format!("{:>width$}", cell.text)
                    } else {
                        format!("{:<width$}", cell.text)
                    };
                    cell.style.paint(&text)
                })
                .collect::<Vec<_>>()
                .join("  ")
        })
        .collect();
    let width = widths.iter().sum::<usize>() + widths.len().saturating_sub(1) * 2;
    (lines, width)
}

fn config_panel(entries: &[registry::Entry], theme: &Theme) -> Result<String> {
    let builtin = ModelRoster::builtin()?.rows();
    let mut text = format!(
        "{}\n{}",
        theme.style(StyleKey::Heading).paint("MODELS"),
        models_table(&builtin, theme)
    );
    for entry in entries {
        writeln!(
            text,
            "\n{}  {}",
            theme.style(StyleKey::Heading).paint(entry.name.as_str()),
            theme.style(StyleKey::Muted).paint(entry.path.as_str())
        )?;
        match project_config(entry, &builtin, theme) {
            Ok(Some(body)) => text.push_str(&body),
            Ok(None) => writeln!(
                text,
                "{} {MANIFEST}",
                theme.style(StyleKey::Lost).paint("✗")
            )?,
            Err(error) => writeln!(
                text,
                "{} {}",
                theme.style(StyleKey::Lost).paint("✗"),
                workspace_manifest::clamp(&format!("{error:#}"), ERROR_WIDTH)
            )?,
        }
    }
    Ok(text)
}

fn project_config(
    entry: &registry::Entry,
    builtin: &[[String; 3]],
    theme: &Theme,
) -> Result<Option<String>> {
    let Some(manifest) = workspace_manifest::load(&entry.path)? else {
        return Ok(None);
    };
    let config = load_project_config_from(&entry.path)?;
    let roles = workspace_manifest::manifest_roles(&manifest, &config);
    let cells = roles
        .iter()
        .map(|row| {
            vec![
                Cell::left(row.role, theme.style(StyleKey::Accent)),
                Cell::left(&row.family, Style::new()),
                Cell::left(&row.model, theme.style(StyleKey::Heading)),
                Cell::left(&row.effort, theme.style(StyleKey::Muted)),
            ]
        })
        .collect::<Vec<_>>();
    let mut text = String::new();
    for (line, row) in table(&cells).0.iter().zip(roles) {
        writeln!(text, "{line}")?;
        if let Some(reason) = row.invalid_reason {
            writeln!(
                text,
                "{}",
                theme
                    .style(StyleKey::Lost)
                    .paint(&format!("  reason: {reason}"))
            )?;
        }
    }
    let mut files = vec![MANIFEST];
    for file in PROJECT_CONFIG_FILES {
        if entry.path.join(file).try_exists()? {
            files.push(file);
        }
    }
    writeln!(
        text,
        "{}",
        files
            .iter()
            .map(|file| format!("{} {file}", theme.style(StyleKey::Running).paint("✓")))
            .collect::<Vec<_>>()
            .join("   ")
    )?;
    let overrides = config
        .models
        .rows()
        .into_iter()
        .filter(|row| !builtin.contains(row))
        .collect::<Vec<_>>();
    if !overrides.is_empty() {
        writeln!(
            text,
            "{}",
            theme
                .style(StyleKey::Muted)
                .paint("models (project overrides)")
        )?;
        text.push_str(&models_table(&overrides, theme));
    }
    Ok(Some(text))
}

fn models_table(rows: &[[String; 3]], theme: &Theme) -> String {
    let mut previous = "";
    let cells = rows
        .iter()
        .map(|[family, model, efforts]| {
            let label = if previous == family { "" } else { family };
            previous = family;
            vec![
                Cell::left(label, theme.style(StyleKey::Accent)),
                Cell::left(workspace_manifest::clamp(model, MODEL_WIDTH), Style::new()),
                Cell::left(efforts, theme.style(StyleKey::Muted)),
            ]
        })
        .collect::<Vec<_>>();
    table(&cells).0.join("\n") + "\n"
}

fn telemetry(projects: &[(String, Vec<SessionUsage>)], theme: &Theme) -> Result<String> {
    let sessions = projects
        .iter()
        .flat_map(|(_, sessions)| sessions)
        .collect::<Vec<_>>();
    if sessions.is_empty() {
        return Ok(format!(
            "{}\n",
            theme.style(StyleKey::Muted).paint("no live sessions")
        ));
    }
    let usages = sessions
        .iter()
        .filter_map(|session| session.usage.as_ref())
        .collect::<Vec<_>>();
    let tokens = usages.iter().map(|usage| usage.total_tokens()).sum();
    let cost = usages
        .iter()
        .filter_map(|usage| usage.estimated_cost_usd)
        .reduce(|a, b| a + b);
    let numbers = |tokens: String, cost: Option<f64>| {
        [
            Cell::right(tokens, theme.style(StyleKey::Heading)),
            Cell::right(
                cost.map_or_else(String::new, |cost| format!("${cost:.2}")),
                theme.style(StyleKey::Muted),
            ),
        ]
    };
    let blanks = |count| std::iter::repeat_with(|| Cell::left("", Style::new())).take(count);
    let mut header = blanks(4).collect::<Vec<_>>();
    header.extend([
        Cell::right("TOKENS", theme.style(StyleKey::Muted)),
        Cell::right("COST", theme.style(StyleKey::Muted)),
    ]);
    let mut cells = vec![header];
    for session in &sessions {
        let state = session
            .usage
            .as_ref()
            .and_then(|usage| usage.state)
            .map(|state| match state {
                SessionState::Working => State::Running,
                SessionState::Waiting => State::Idle,
            });
        let (glyph, style) = match state {
            Some(state) => theme.state(state),
            None => (" ", Style::new()),
        };
        let id = if session.role == "lead" {
            "lead"
        } else {
            &session.id
        };
        let mut row = vec![
            Cell::left(glyph, style),
            Cell::left(id, theme.style(StyleKey::Accent)),
            Cell::left(session.role, theme.style(StyleKey::Muted)),
            Cell::left(&session.agent, Style::new()),
        ];
        row.extend(numbers(
            session.usage.as_ref().map_or_else(
                || "?".into(),
                |usage| rows::abbreviate(usage.total_tokens()),
            ),
            session
                .usage
                .as_ref()
                .and_then(|usage| usage.estimated_cost_usd),
        ));
        cells.push(row);
    }
    let mut total = blanks(1).collect::<Vec<_>>();
    total.push(Cell::left("total", theme.style(StyleKey::Heading)));
    total.extend(blanks(2));
    total.extend(numbers(rows::abbreviate(tokens), cost));
    cells.push(total);
    let (lines, width) = table(&cells);
    let mut text = format!("  {}\n", lines[0]);
    let mut index = 1;
    for (name, sessions) in projects.iter().filter(|(_, sessions)| !sessions.is_empty()) {
        writeln!(text, "{}", theme.style(StyleKey::Heading).paint(name))?;
        for session in sessions {
            write!(text, "  {}", lines[index])?;
            if session.window_gone {
                write!(
                    text,
                    "  {}",
                    theme.style(StyleKey::Lost).paint("window lost")
                )?;
            }
            text.push('\n');
            index += 1;
        }
    }
    writeln!(
        text,
        "{}\n  {}",
        theme.style(StyleKey::Guide).paint(&"─".repeat(width + 2)),
        lines[index]
    )?;
    Ok(text)
}

#[cfg(test)]
mod tests;
