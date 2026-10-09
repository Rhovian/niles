use anyhow::{Context, Result};
use clap::{Subcommand, ValueEnum};
use ratatui::style::Style;

use super::registry::{self, ProjectName};
use crate::{
    session,
    theme::{State, StyleKey, StyleRender, Theme},
    tmux,
    worker::usage::{self, SessionUsage},
};

mod config;
mod telemetry;

pub(crate) use telemetry::Range;

#[derive(Clone, Debug, PartialEq, Eq, Subcommand)]
pub(crate) enum Panel {
    /// The global config, or `--project`'s.
    Config {
        #[arg(long, value_parser = ProjectName::parse)]
        project: Option<ProjectName>,
    },
    Telemetry {
        #[arg(long, value_enum)]
        range: Range,
    },
    Help,
}

impl Panel {
    /// The panel's window, which every range of the telemetry panel shares.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Config { .. } => "config",
            Self::Telemetry { .. } => "telemetry",
            Self::Help => "help",
        }
    }
}

/// Shows `panel` in its window, replacing whatever that window showed.
pub(crate) fn open(panel: Panel) -> Result<tmux::WindowTarget> {
    match &panel {
        Panel::Telemetry { range } => {
            let range = range
                .to_possible_value()
                .context("every range has a --range value")?;
            tmux::open_panel(panel.name(), &["--range", range.get_name()])
        }
        Panel::Config { project: None } => tmux::open_panel(panel.name(), &[]),
        Panel::Config {
            project: Some(name),
        } => tmux::open_panel(panel.name(), &["--project", name.as_str()]),
        Panel::Help => tmux::open_panel(panel.name(), &[]),
    }
}

pub(crate) fn render(panel: Panel) -> Result<()> {
    let theme = Theme::load()?;
    let text = match panel {
        Panel::Help => help_text(registry::entries()?.is_empty(), &theme),
        // Interactive, so it draws for itself until its window is respawned.
        Panel::Config { project } => return config::run(project.as_ref(), &theme),
        Panel::Telemetry { range } => telemetry::panel(&registry::entries()?, range, &theme)?,
    };
    print!("{text}");
    Ok(())
}

/// The project's workers, and its lead while it runs.
pub(super) fn open_sessions(entry: &registry::Entry) -> Result<Vec<SessionUsage>> {
    let lead = if lead_running(entry)? {
        session::latest_lead(&entry.path)?
    } else {
        None
    };
    usage::collect(&entry.path, lead)
}

fn lead_running(entry: &registry::Entry) -> Result<bool> {
    let session = entry.name.session()?;
    Ok(
        tmux::project_session(&session)?.as_deref() == Some(entry.path.as_str())
            && tmux::lead_running(&session)?,
    )
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

#[cfg(test)]
mod tests;
