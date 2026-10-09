use std::fmt::Write;

use anyhow::{Context, Result};
use clap::{Subcommand, ValueEnum};
use ratatui::style::Style;

use super::registry;
use crate::{
    agents::ModelRoster,
    config::spec::{PROJECT_CONFIG_FILES, load_project_config_from},
    session,
    theme::{State, StyleKey, StyleRender, Theme},
    tmux,
    worker::usage::{self, SessionUsage},
    workspace_manifest,
};

mod telemetry;

pub(crate) use telemetry::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Subcommand)]
pub(crate) enum Panel {
    Config,
    Telemetry {
        #[arg(long, value_enum)]
        range: Range,
    },
    Help,
}

impl Panel {
    /// The panel's window, which every range of the telemetry panel shares.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Telemetry { .. } => "telemetry",
            Self::Help => "help",
        }
    }
}

/// Shows `panel` in its window, replacing whatever that window showed.
pub(crate) fn open(panel: Panel) -> Result<tmux::WindowTarget> {
    match panel {
        Panel::Telemetry { range } => {
            let range = range
                .to_possible_value()
                .context("every range has a --range value")?;
            tmux::open_panel(panel.name(), &["--range", range.get_name()])
        }
        Panel::Config | Panel::Help => tmux::open_panel(panel.name(), &[]),
    }
}

pub(crate) fn render(panel: Panel) -> Result<()> {
    let theme = Theme::load()?;
    let mut text = String::new();
    let entries = registry::entries()?;
    match panel {
        Panel::Help => text.push_str(&help_text(entries.is_empty(), &theme)),
        Panel::Config => text.push_str(&config_panel(&entries, &theme)?),
        Panel::Telemetry { range } => text.push_str(&telemetry::panel(&entries, range, &theme)?),
    }
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

#[cfg(test)]
mod tests;
