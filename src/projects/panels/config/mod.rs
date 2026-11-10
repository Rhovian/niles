//! The CONFIG panel: one scope's settings as key / value / from. ↵ runs the tier picker on a
//! project tier; ←/→ steps simple values in place. Other settings are read-only.

use self::items::{Edit, Item, Note, Step, Tier};
use super::registry::{self, ProjectName};
use crate::agents::picker;
use crate::config::spec::{ProjectConfig, load_project_config_from};
use crate::config::user;
use crate::selection::{self, Selection};
use crate::theme::{StyleKey, Theme};
use crate::tmux;
use crate::workspace_manifest::{self, WorkspaceManifest};
use anyhow::{Context, Result};
use camino::Utf8Path;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Row, Table, TableState, Wrap};
use ratatui::{DefaultTerminal, Frame};
use ratatui_themes::ThemeName;

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
        items: Selection::new(items(project)?).context("config has no rows")?,
        footer: None,
        theme: theme.clone(),
    };
    panel.items.prefer(|item| item.edit().is_some());
    let mut terminal = ratatui::init();
    let result = panel.run(&mut terminal);
    ratatui::restore();
    result
}

struct ConfigPanel<'a> {
    project: Option<&'a ProjectName>,
    items: Selection<Item>,
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
                _ => continue,
            }
        }
    }

    fn act(&mut self, terminal: &mut DefaultTerminal, key: KeyCode) -> Result<()> {
        let Item::Setting(setting) = self.items.current() else {
            return Ok(());
        };
        let theme = &self.theme;
        self.footer = match (setting.edit.as_ref(), key) {
            (Some(Edit::Registry), KeyCode::Enter) => Some(format!(
                "r in the explorer registers; select {} and press d to deregister",
                setting.key
            )),
            (Some(Edit::Tier { root, tier }), KeyCode::Enter) => {
                pick_tier(terminal, root, tier, theme).err().map(shown)
            }
            (Some(Edit::Step(step)), KeyCode::Enter | KeyCode::Left | KeyCode::Right) => step
                .save(cycle(
                    &step.options(),
                    &setting.value,
                    key == KeyCode::Left,
                )?)
                .and_then(|()| Theme::load())
                .map(|theme| self.theme = theme)
                .err()
                .map(shown),
            _ => return Ok(()),
        };
        let position = self.items.position();
        self.items = Selection::new(items(self.project)?).context("config has no rows")?;
        if self.items.focus(position).is_err() || self.items.current().edit().is_none() {
            self.items.prefer(|item| item.edit().is_some());
        }
        Ok(())
    }

    fn up(&mut self) {
        self.items
            .move_matching(false, |item| item.edit().is_some());
    }
    fn down(&mut self) {
        self.items.move_matching(true, |item| item.edit().is_some());
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
        let (key_width, value_width) =
            rows.iter()
                .fold((0, 0), |(key_width, value_width), [key, value, _]| {
                    (key_width.max(key.width()), value_width.max(value.width()))
                });
        #[expect(clippy::disallowed_methods, reason = "frame clipping")]
        let width = |width: usize| Constraint::Length(u16::try_from(width).unwrap_or(u16::MAX));
        let widths = [width(key_width), width(value_width), Constraint::Fill(1)];
        let table = Table::new(rows.into_iter().map(Row::new), widths)
            .column_spacing(2)
            .row_highlight_style(theme.style(StyleKey::Selection));
        let mut state = TableState::default()
            .with_selected(self.items.current().edit().map(|_| self.items.position()));
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

fn cycle<'a>(options: &[&'a str], current: &str, previous: bool) -> Result<&'a str> {
    let index = options.iter().position(|value| *value == current);
    let index = if index.is_none() && previous {
        Some(0)
    } else {
        index
    };
    selection::cycle(options, index, if previous { -1 } else { 1 })
        .copied()
        .context("config setting has no options")
}

fn pick_tier(
    terminal: &mut DefaultTerminal,
    root: &Utf8Path,
    tier: &Tier,
    theme: &Theme,
) -> Result<()> {
    edit_tiers(root, *tier, |manifest, config, tier| {
        picker::configure(terminal, root, manifest, config, theme, tier)
    })
}

fn edit_tiers(
    root: &Utf8Path,
    tier: Tier,
    pick: impl FnOnce(WorkspaceManifest, &ProjectConfig, Tier) -> Result<picker::Choice>,
) -> Result<()> {
    let config = load_project_config_from(root)?;
    let manifest = load_manifest(root)?;
    match pick(manifest.clone(), &config, tier)? {
        picker::Choice::Save(draft) => {
            workspace_manifest::save_changes(root, None, &manifest, &draft)?;
        }
        picker::Choice::Keep | picker::Choice::Quit => {}
    }
    Ok(())
}

/// Sets a field, through `set`, in the manifest as it is now, so the picker's wait loses nothing.
/// race accepted: a concurrent edit in that window is lost, last writer wins.
fn save_manifest(root: &Utf8Path, set: impl FnOnce(&mut WorkspaceManifest)) -> Result<()> {
    let mut manifest = load_manifest(root)?;
    set(&mut manifest);
    workspace_manifest::save(root, &manifest, None)
}

fn load_manifest(root: &Utf8Path) -> Result<WorkspaceManifest> {
    let path = workspace_manifest::manifest_path(root, None)?;
    workspace_manifest::load(root, None)?.with_context(|| format!("{path} is gone"))
}
