//! The CONFIG panel: one scope's settings as key / value / from. ↵ runs the role picker on a
//! single-model role and `$EDITOR` on the file behind any other row.

use std::{
    env,
    ffi::{OsStr, OsString},
    process::Command,
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout},
    text::{Line, Span},
    widgets::{Paragraph, Row, Table, TableState, Wrap},
};

use self::items::{Edit, Item, Note, Role};
use super::registry::{self, ProjectName};
use crate::{
    agents::picker,
    config::{spec::load_project_config_from, user},
    theme::{StyleKey, Theme},
    tmux,
    workspace_manifest::{self, RoleBinding, WorkspaceManifest},
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
        editor: env::var_os("EDITOR").filter(|editor| !editor.is_empty()),
    };
    panel.down();
    let mut terminal = ratatui::init();
    let result = panel.run(&mut terminal, theme);
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
    /// `$EDITOR`, unless it is unset or empty.
    editor: Option<OsString>,
}

impl ConfigPanel<'_> {
    /// Runs until tmux respawns the panel's window.
    fn run(&mut self, terminal: &mut DefaultTerminal, theme: &Theme) -> Result<()> {
        loop {
            terminal.draw(|frame| self.draw(frame, theme))?;
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
                (KeyEventKind::Press, KeyCode::Enter) => self.enter(terminal)?,
                _ => {}
            }
        }
    }

    /// Edits the selected row, then reloads every row, since an edit can change any of them.
    fn enter(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let Some(edit) = self.selected.and_then(|index| self.items[index].edit()) else {
            return Ok(());
        };
        self.footer = match edit.clone() {
            Edit::Registry { name } => Some(format!(
                "r in the explorer registers; rm ~/.niles/projects/{name} unregisters"
            )),
            Edit::File(file) => match self.editor.as_deref() {
                Some(editor) => suspended(terminal, || edit_file(Some(editor), &file)),
                // Nothing runs, so the terminal stays the panel's.
                None => edit_file(None, &file),
            }
            .err()
            .map(shown),
            Edit::Role { root, role } => suspended(terminal, || pick_role(&root, role))
                .err()
                .map(shown),
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

    fn draw(&self, frame: &mut Frame, theme: &Theme) {
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
        Item::Broken { reason, .. } => [
            Line::styled("  ✗", theme.style(StyleKey::Lost)),
            Line::raw(reason.as_str()),
            Line::default(),
        ],
    }
}

/// Hands the terminal to `action`, then takes it back.
fn suspended<T>(terminal: &mut DefaultTerminal, action: impl FnOnce() -> Result<T>) -> Result<T> {
    ratatui::restore();
    let result = action();
    *terminal = ratatui::init();
    result
}

fn shown(error: anyhow::Error) -> String {
    format!("{error:#}")
}

/// Opens `file` in `editor`, through `sh` so an editor with arguments (`code -w`) works.
fn edit_file(editor: Option<&OsStr>, file: &Utf8Path) -> Result<()> {
    let Some(editor) = editor else {
        bail!("set $EDITOR to edit {file}");
    };
    let status = Command::new("sh")
        .args(["-c", "$EDITOR \"$1\"", "sh", file.as_str()])
        .env("EDITOR", editor)
        .status()
        .context("failed to run $EDITOR")?;
    if !status.success() {
        bail!("$EDITOR exited with {status}");
    }
    Ok(())
}

fn pick_role(root: &Utf8Path, role: Role) -> Result<()> {
    let config = load_project_config_from(root)?;
    let manifest = load_manifest(root)?;
    let agent = |label, binding: &RoleBinding| {
        picker::prompt_agent_value(label, binding.default_model(), &config).map(RoleBinding::from)
    };
    match role {
        Role::Lead => {
            let lead = picker::prompt_agent_value("Lead agent", &manifest.lead, &config)?;
            save_role(root, |manifest| manifest.lead = lead)
        }
        Role::Worker => {
            let worker = agent("Worker agent", &manifest.worker)?;
            save_role(root, |manifest| manifest.worker = worker)
        }
        Role::Reviewer => {
            let reviewer =
                picker::prompt_reviewer_value("Reviewer agent", &manifest.reviewer, &config)?;
            save_role(root, |manifest| manifest.reviewer = reviewer)
        }
        Role::Security => {
            let security = agent("Security agent", &manifest.security)?;
            save_role(root, |manifest| manifest.security = security)
        }
    }
}

/// Sets one role, through `set`, in the manifest as it is now, so the picker's wait loses nothing.
/// race accepted: a concurrent edit in that window is lost, last writer wins.
fn save_role(root: &Utf8Path, set: impl FnOnce(&mut WorkspaceManifest)) -> Result<()> {
    let mut manifest = load_manifest(root)?;
    set(&mut manifest);
    workspace_manifest::save(root, &manifest)
}

fn load_manifest(root: &Utf8Path) -> Result<WorkspaceManifest> {
    workspace_manifest::load(root)?
        .with_context(|| format!("{} is gone", workspace_manifest::manifest_path(root)))
}
