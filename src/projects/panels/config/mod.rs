//! The CONFIG panel: one scope's settings as key / value / from. ↵ runs the role picker on a
//! single-model role; ←/→ steps simple values in place. Other settings are read-only.

use anyhow::{Context, Result};
use camino::Utf8Path;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout},
    text::{Line, Span},
    widgets::{Paragraph, Row, Table, TableState, Wrap},
};
use ratatui_themes::ThemeName;

use self::items::{Edit, Item, Note, Role, Step};
use super::registry::{self, ProjectName};
use crate::{
    agents::picker,
    config::{spec::load_project_config_from, user},
    theme::{StyleKey, Theme},
    tmux,
    workspace_manifest::{self, WorkspaceManifest},
};

mod items;
#[cfg(test)]
mod tests;

/// The rows for `project`'s config, or the global config's when there is no project.
fn items(project: Option<&ProjectName>) -> Result<Vec<Item>> {
    let entries = registry::entries()?;
    let Some(name) = project else {
        return Ok(items::global(&user::path()?, &entries));
    };
    let entry = entries
        .iter()
        .find(|entry| entry.name == *name)
        .with_context(|| format!("{} is not registered", name.as_str()))?;
    items::project(&entry.path)
}

pub(super) fn run(project: Option<&ProjectName>, theme: &Theme) -> Result<()> {
    let mut panel = ConfigPanel {
        project,
        items: items(project)?,
        selected: None,
        footer: None,
        theme: theme.clone(),
    };
    panel.down();
    let mut terminal = ratatui::init();
    let result = panel.run(&mut terminal);
    ratatui::restore();
    result
}

struct ConfigPanel<'a> {
    project: Option<&'a ProjectName>,
    items: Vec<Item>,
    /// Always a row with an edit; `None` only when no row has one.
    selected: Option<usize>,
    /// The outcome of the last ↵.
    footer: Option<String>,
    theme: Theme,
}

impl ConfigPanel<'_> {
    /// Runs until tmux respawns the panel's window.
    fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind == KeyEventKind::Press {
                self.footer = None;
            }
            match (key.kind, key.code) {
                (KeyEventKind::Press, KeyCode::Up) => self.up(),
                (KeyEventKind::Press, KeyCode::Down) => self.down(),
                (KeyEventKind::Press, KeyCode::Esc) => {
                    self.footer = tmux::focus_explorer().err().map(shown);
                }
                (KeyEventKind::Press, KeyCode::Enter | KeyCode::Left | KeyCode::Right) => {
                    self.act(terminal, key.code)?
                }
                _ => {}
            }
        }
    }

    fn act(&mut self, terminal: &mut DefaultTerminal, key: KeyCode) -> Result<()> {
        let Some(Item::Setting(setting)) = self.selected.map(|index| &self.items[index]) else {
            return Ok(());
        };
        let theme = &self.theme;
        self.footer = match (setting.edit.as_ref(), key) {
            (Some(Edit::Registry), KeyCode::Enter) => Some(format!(
                "r in the explorer registers; rm ~/.niles/projects/{} unregisters",
                setting.key
            )),
            (Some(Edit::Role { root, role }), KeyCode::Enter) => {
                pick_role(terminal, root, *role, theme).err().map(shown)
            }
            (Some(Edit::Step(step)), KeyCode::Enter | KeyCode::Left | KeyCode::Right) => step
                .save(cycle(&step.options(), &setting.value, key == KeyCode::Left))
                .and_then(|()| Theme::load())
                .map(|theme| self.theme = theme)
                .err()
                .map(shown),
            _ => return Ok(()),
        };
        self.items = items(self.project)?;
        if self
            .selected
            .and_then(|index| self.items.get(index)?.edit())
            .is_none()
        {
            self.selected = None;
            self.down();
        }
        Ok(())
    }

    fn up(&mut self) {
        if let Some(index) = self.selected
            && let Some(above) = (0..index).rev().find(|&index| self.editable(index))
        {
            self.selected = Some(above);
        }
    }

    /// Moves to the next row with an edit, or the first one when none is selected.
    fn down(&mut self) {
        let from = self.selected.map_or(0, |index| index + 1);
        if let Some(below) = (from..self.items.len()).find(|&index| self.editable(index)) {
            self.selected = Some(below);
        }
    }

    fn editable(&self, index: usize) -> bool {
        self.items[index].edit().is_some()
    }

    fn draw(&self, frame: &mut Frame) {
        let theme = &self.theme;
        let [body, footer] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(2)]).areas(frame.area());
        let rows = self
            .items
            .iter()
            .map(|item| cells(item, theme))
            .collect::<Vec<_>>();
        let width = |column: usize| {
            let width = rows
                .iter()
                .map(|row| row[column].width())
                .fold(0, usize::max);
            #[expect(
                clippy::disallowed_methods,
                reason = "a column past u16::MAX is clipped to the frame either way"
            )]
            let width = u16::try_from(width).unwrap_or(u16::MAX);
            Constraint::Length(width)
        };
        let widths = [width(0), width(1), Constraint::Fill(1)];
        let table = Table::new(rows.into_iter().map(Row::new), widths)
            .column_spacing(2)
            .row_highlight_style(theme.style(StyleKey::Selection));
        let mut state = TableState::default().with_selected(self.selected);
        frame.render_stateful_widget(table, body, &mut state);
        if let Some(message) = &self.footer {
            frame.render_widget(
                Paragraph::new(message.as_str()).wrap(Wrap { trim: false }),
                footer,
            );
        }
    }
}

/// The key, value and from cells of `item`'s row.
fn cells<'a>(item: &'a Item, theme: &Theme) -> [Line<'a>; 3] {
    match item {
        Item::Section(name) => [
            Line::styled(name.as_str(), theme.style(StyleKey::Heading)),
            Line::default(),
            Line::default(),
        ],
        Item::Setting(setting) => {
            let mut from = Line::styled(setting.from, theme.style(StyleKey::Muted));
            match &setting.note {
                Note::None => {}
                Note::Builtin(value) => from.push_span(Span::styled(
                    format!(" (builtin: {value})"),
                    theme.style(StyleKey::Muted),
                )),
                Note::Invalid(reason) => from.push_span(Span::styled(
                    format!(" ✗ {reason}"),
                    theme.style(StyleKey::Lost),
                )),
            }
            [
                Line::styled(format!("  {}", setting.key), theme.style(StyleKey::Accent)),
                Line::raw(setting.value.as_str()),
                from,
            ]
        }
        Item::Broken(reason) => [
            Line::styled("  ✗", theme.style(StyleKey::Lost)),
            Line::raw(reason.as_str()),
            Line::default(),
        ],
    }
}

fn shown(error: anyhow::Error) -> String {
    format!("{error:#}")
}

fn step_config(path: &Utf8Path, set: impl FnOnce(&mut user::FileConfig)) -> Result<()> {
    let mut file = user::FileConfig::load(path)?;
    set(&mut file);
    file.save(path)
}

impl Step {
    fn options(&self) -> Vec<&'static str> {
        match self {
            Self::Theme(_) => ThemeName::all().iter().map(|name| name.slug()).collect(),
            Self::Bindings(_) => vec!["false", "true"],
            Self::Checkin(_) => CHECKIN_STEPS.to_vec(),
            Self::Recheck(_) => RECHECK_STEPS.to_vec(),
        }
    }

    fn save(&self, text: &str) -> Result<()> {
        match self {
            Self::Theme(path) => {
                let theme = text.parse::<ThemeName>().map_err(anyhow::Error::msg)?;
                step_config(path, |file| file.theme = Some(theme))
            }
            Self::Bindings(path) => {
                let bindings = text.parse::<bool>()?;
                step_config(path, |file| file.tmux.bindings = Some(bindings))
            }
            Self::Checkin(root) => save_manifest(root, |file| file.checkin = Some(text.to_owned())),
            Self::Recheck(root) => save_manifest(root, |file| file.recheck = Some(text.to_owned())),
        }
    }
}

const CHECKIN_STEPS: &[&str] = &["off", "2m", "5m", "10m", "15m", "30m", "1h"];
const RECHECK_STEPS: &[&str] = &["backoff", "5m", "10m", "15m", "30m", "1h"];

fn cycle<'a>(options: &[&'a str], current: &str, previous: bool) -> &'a str {
    debug_assert!(!options.is_empty());
    let next = match (options.iter().position(|value| *value == current), previous) {
        (Some(index), true) => (index + options.len() - 1) % options.len(),
        (Some(index), false) => (index + 1) % options.len(),
        (None, true) => options.len() - 1,
        (None, false) => 0,
    };
    options[next]
}

fn pick_role(
    terminal: &mut DefaultTerminal,
    root: &Utf8Path,
    role: Role,
    theme: &Theme,
) -> Result<()> {
    let config = load_project_config_from(root)?;
    let manifest = load_manifest(root)?;
    if let Some(value) = picker::role(terminal, role, &manifest, &config, theme)?
        && value != role.value(&manifest)
    {
        save_manifest(root, |manifest| role.set(manifest, value))?;
    }
    Ok(())
}

/// Sets a field, through `set`, in the manifest as it is now, so the picker's wait loses nothing.
/// race accepted: a concurrent edit in that window is lost, last writer wins.
fn save_manifest(root: &Utf8Path, set: impl FnOnce(&mut WorkspaceManifest)) -> Result<()> {
    let mut manifest = load_manifest(root)?;
    set(&mut manifest);
    workspace_manifest::save(root, &manifest)
}

fn load_manifest(root: &Utf8Path) -> Result<WorkspaceManifest> {
    workspace_manifest::load(root)?
        .with_context(|| format!("{} is gone", workspace_manifest::manifest_path(root)))
}
